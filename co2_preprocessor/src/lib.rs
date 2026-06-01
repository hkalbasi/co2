use std::collections::HashMap;
use std::fs;
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
    files: Arc<HashMap<FileId, SourceFile>>,
    main_rewrite_boundaries: Arc<Vec<usize>>,
}

impl PreprocessedSource {
    pub fn files(&self) -> &HashMap<FileId, SourceFile> {
        &self.files
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
    Span::from_parts(*file_id, start..end)
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

#[derive(Debug)]
struct MappedText {
    text: String,
    boundaries: Vec<usize>,
}
