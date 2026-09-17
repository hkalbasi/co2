#![feature(rustc_private)]

extern crate rustc_data_structures;

use rustc_data_structures::fx::FxHashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod builtin_macros;
mod conditionals;
mod expr_eval;
mod includes;
mod macro_defs;
mod macro_token;
mod pipeline;
mod pragmas;
mod predefined_macros;
mod text_processing;
mod tokenizer;
mod utils;

use co2_ast::{FileId, Rich, SourceMap, Span, Spanned, Token};
use pipeline::Preprocessor;

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct PreprocessedSource {
    pub raw_src: Arc<str>,
    pub tokens: Arc<Vec<Spanned<Token>>>,
    pub main_file_idx: FileId,
    files: Arc<FxHashMap<FileId, SourceFile>>,
    main_rewrite_boundaries: Arc<Vec<usize>>,
}

impl PreprocessedSource {
    pub fn files(&self) -> &FxHashMap<FileId, SourceFile> {
        &self.files
    }
}

struct PreprocessorSourceMap {
    files: Arc<FxHashMap<FileId, (String, Arc<str>)>>,
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
    let input_source = prepend_co2_define(&input_bytes);
    let source = preprocessor.preprocess(&input_source.text, &input_source.boundaries);
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
        return Span::from_parts(preprocessed.main_file_idx, 0..0);
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

    if let Some((logical_file, logical_line)) = &diagnostic.logical
        && let Some(span) = translate_logical_span(
            preprocessed,
            *file_id,
            file,
            start..end,
            logical_file,
            *logical_line,
        )
    {
        return span;
    }

    Span::from_parts(*file_id, start..end)
}

/// Re-anchor a diagnostic span through a `#line` remap: keep the columns of
/// the physical range but move to `logical_line` of the logical file,
/// computing fresh byte offsets against that file's content. Returns `None`
/// when the logical file is unknown, in which case the directive is ignored
/// for span purposes and the physical span is used.
fn translate_logical_span(
    preprocessed: &PreprocessedSource,
    physical_file_id: FileId,
    physical: &SourceFile,
    physical_range: Range<usize>,
    logical_file: &str,
    logical_line: u64,
) -> Option<Span> {
    let logical_path = absolute_path(Path::new(logical_file));
    let (file_id, source): (FileId, Arc<str>) = if logical_path == physical.path {
        (physical_file_id, physical.source.clone())
    } else {
        let (id, file) = preprocessed
            .files()
            .iter()
            .find(|(_, file)| file.path == logical_path)?;
        (*id, file.source.clone())
    };

    // Columns of the physical range within its own line.
    let phys = &physical.source;
    let phys_line_start = phys[..physical_range.start]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let col_start = physical_range.start - phys_line_start;
    let col_end = physical_range
        .end
        .saturating_sub(phys_line_start)
        .max(col_start);

    // Byte offset of the logical line ("line index as byte offset"), clamped
    // to the last line of the logical file.
    let log = &source;
    let total_lines = 1 + log.bytes().filter(|b| *b == b'\n').count();
    let target = logical_line.clamp(1, total_lines as u64) as usize;
    let mut log_line_start = 0usize;
    for _ in 1..target {
        match log[log_line_start..].find('\n') {
            Some(i) => log_line_start += i + 1,
            None => break,
        }
    }

    let start = (log_line_start + col_start).min(log.len());
    let mut end = (log_line_start + col_end).min(log.len()).max(start);
    if end == start && start < log.len() {
        end += 1;
    }
    Some(Span::from_parts(file_id, start..end))
}

/// Detect an available C compiler driver, honoring `$CO2_CC`.
/// Returns the first of `cc`/`gcc`/`clang` found on PATH.
pub fn detect_c_compiler() -> Option<String> {
    if let Ok(cc) = std::env::var("CO2_CC") {
        return Some(cc);
    }
    let candidates = ["cc", "gcc", "clang"];
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for candidate in candidates {
            if dir.join(candidate).is_file() {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

pub fn discover_system_include_paths() -> Vec<PathBuf> {
    // Probe whichever C compiler is installed; gcc and clang both print their
    // `#include <...>` search list to stderr under `-E -v`.
    let Some(compiler) = detect_c_compiler() else {
        return Vec::new();
    };

    let Ok(output) = std::process::Command::new(&compiler)
        .args(["-E", "-v", "-"])
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

    let mut paths = Vec::new();
    let stderr = String::from_utf8_lossy(&output.stderr);
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
            "-nostdinc" | "-undef" => {}
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

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .expect("failed to resolve current dir")
        .join(path)
}

fn prepend_co2_define(source: &str) -> MappedText {
    let prefix = "#define __CO2__ 1\n";
    let mut rewritten = String::with_capacity(prefix.len() + source.len());
    rewritten.push_str(prefix);
    rewritten.push_str(source);
    // Rewritten offsets [0..=prefix.len()] map to original offset 0 (the
    // injected define); every original byte i maps to itself (shifted by the
    // prefix length in the rewritten text).
    let mut boundaries = vec![0; prefix.len() + 1];
    boundaries.extend(1..=source.len());
    MappedText {
        text: rewritten,
        boundaries,
    }
}

#[derive(Debug)]
struct MappedText {
    text: String,
    boundaries: Vec<usize>,
}
