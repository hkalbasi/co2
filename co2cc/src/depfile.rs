use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::args::DepFileArgs;

fn quote_for_make(s: &str) -> String {
    s.replace('$', "$$")
}

fn system_include_dirs(cpp_args: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !cpp_args.iter().any(|a| a == "-nostdinc") {
        dirs.extend(co2_preprocessor::discover_system_include_paths());
    }
    dirs.extend(cpp_args.iter().enumerate().filter_map(|(i, arg)| {
        if arg == "-isystem" {
            cpp_args.get(i + 1).map(PathBuf::from)
        } else {
            arg.strip_prefix("-isystem")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        }
    }));
    dirs
}

pub(crate) fn write_dep_file(
    preprocessed: &co2_preprocessor::PreprocessedSource,
    obj_output: Option<&Path>,
    dep_args: &DepFileArgs,
    cpp_args: &[String],
) {
    if !dep_args.generate {
        return;
    }

    let target = dep_args.target.as_deref().map_or_else(
        || {
            obj_output
                .map_or_else(|| "a.out".to_owned(), |p| p.to_string_lossy().into_owned())
        },
        |t| {
            if dep_args.target_quoted {
                quote_for_make(t)
            } else {
                t.to_owned()
            }
        },
    );

    let dep_path = dep_args.output.clone().unwrap_or_else(|| {
        obj_output.map_or_else(|| PathBuf::from("a.d"), |p| p.with_extension("d"))
    });

    let mut deps: Vec<String> = preprocessed
        .files()
        .values()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect();
    deps.sort();
    if dep_args.exclude_system {
        let system_dirs = system_include_dirs(cpp_args);
        deps.retain(|dep| !system_dirs.iter().any(|d| Path::new(dep).starts_with(d)));
    }

    let mut content = format!("{target}:");
    for dep in &deps {
        let _ = write!(content, " \\\n  {dep}");
    }
    content.push('\n');

    if dep_args.phony {
        for dep in &deps {
            let _ = write!(content, "\n{dep}:\n");
        }
    }

    if let Some(parent) = dep_path.parent() {
        fs::create_dir_all(parent).expect("failed to create depfile parent directory");
    }
    fs::write(&dep_path, &content).expect("failed to write dependency file");
}
