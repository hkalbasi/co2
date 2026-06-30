use clap::Parser;
use globset::{Glob, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

mod config;
mod error;
mod formatter;
mod lexer;
mod token;

use config::Config;

static C_EXTENSIONS: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx"];

#[derive(Debug, Clone, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
enum EmitMode {
    Stdout,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
enum ColorWhen {
    Always,
    Never,
    Auto,
}

#[derive(Parser)]
#[command(
    name = "funky",
    about = "C/C++ formatter with Unicode support"
)]
struct Cli {
    /// Source file(s) or director(ies) to format. Use `-` to read from stdin. If no file is given, reads from stdin.
    #[arg()]
    files: Vec<PathBuf>,

    /// Path to the configuration file
    #[arg(long = "config-path", short = 'c', value_name = "PATH")]
    config_path: Option<PathBuf>,

    /// What data to emit and how: `stdout` (default) or `files`
    #[arg(long, value_name = "MODE", default_value = "stdout", value_parser = clap::value_parser!(EmitMode))]
    emit: EmitMode,

    /// Run in 'check' mode. Exits with 0 if input is formatted correctly.
    #[arg(long)]
    check: bool,

    /// Ignored
    #[arg(long)]
    edition: Option<String>,

    /// Ignored
    #[arg(long, value_name = "WHEN", default_value = "auto", value_parser = clap::value_parser!(ColorWhen))]
    color: ColorWhen,

    /// Prints the names of files that would be reformatted
    #[arg(short = 'l', long)]
    files_with_diff: bool,

    /// Ignored
    #[arg(long, value_name = "KEY=VAL", value_delimiter = ',')]
    config: Vec<String>,

    /// Ignored
    #[arg(short, long)]
    verbose: bool,

    /// Ignored
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Recurse into directories and format all C/C++ source files found.
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Print the raw token stream and exit (for debugging).
    #[arg(long, hide = true)]
    dump_tokens: bool,
}

pub fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        let version = std::env::var("CO2_VERSION").unwrap_or_else(|_| "unknown".to_owned());
        println!("co2fmt {version}");
        return Ok(());
    }
    let cli = Cli::parse();

    if cli.check && cli.emit == EmitMode::Files {
        anyhow::bail!("--check and --emit=files are mutually exclusive");
    }

    let config = load_config(cli.config_path.as_deref())?;
    let expanded = if cli.files.is_empty() {
        vec![PathBuf::from("-")]
    } else {
        expand_paths(&cli.files, cli.recursive, &config)?
    };

    let mut any_changed = false;

    for path in &expanded {
        let is_stdin = path == Path::new("-");
        let source = read_source(path)?;
        let (tokens, warnings) = lexer::tokenize(&source, path.display().to_string())?;
        for w in &warnings {
            eprintln!("warning: {w}");
        }

        if cli.dump_tokens {
            for tok in &tokens {
                println!("{:?} {:?}", tok.kind, tok.lexeme);
            }
            continue;
        }

        let formatted = formatter::format(&tokens, &config)?;

        if cli.check {
            if source != formatted {
                if cli.files_with_diff && !is_stdin {
                    println!("{}", path.display());
                } else {
                    eprintln!("{}: would reformat", path.display());
                }
                any_changed = true;
            }
        } else if cli.emit == EmitMode::Files && !is_stdin {
            if source != formatted {
                std::fs::write(path, formatted.as_bytes())
                    .map_err(|e| anyhow::anyhow!("could not write {}: {}", path.display(), e))?;
            }
        } else {
            print!("{}", formatted);
        }
    }

    if cli.check && any_changed {
        std::process::exit(1);
    }

    Ok(())
}

/// Expand the raw CLI paths into a flat list of files to process.
///
/// - Plain files are passed through unchanged.
/// - `-` (stdin) is passed through unchanged.
/// - Directories require `--recursive`; without it, passing a directory is an error.
/// - When recursing, files are filtered to C/C++ extensions and against the
///   ignore patterns from `config.ignore.patterns`.
fn expand_paths(
    inputs: &[PathBuf],
    recursive: bool,
    config: &Config,
) -> anyhow::Result<Vec<PathBuf>> {
    let ignore = build_ignore_set(&config.ignore.patterns)?;
    let mut out = Vec::new();

    for input in inputs {
        if input == Path::new("-") {
            out.push(input.clone());
            continue;
        }

        let meta = std::fs::metadata(input)
            .map_err(|e| anyhow::anyhow!("could not stat {}: {}", input.display(), e))?;

        if meta.is_file() {
            // Apply ignore patterns to directly-passed files too, matching
            // against both the full path and just the filename component.
            let ignored = ignore.is_match(input)
                || input
                    .file_name()
                    .map(|n| ignore.is_match(Path::new(n)))
                    .unwrap_or(false);
            if !ignored {
                out.push(input.clone());
            }
            continue;
        }

        if meta.is_dir() {
            if !recursive {
                anyhow::bail!(
                    "{} is a directory — use --recursive to format directory trees",
                    input.display()
                );
            }
            collect_dir(input, &ignore, &mut out);
            continue;
        }

        anyhow::bail!("{}: not a file or directory", input.display());
    }

    Ok(out)
}

fn collect_dir(root: &Path, ignore: &globset::GlobSet, out: &mut Vec<PathBuf>) {
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !C_EXTENSIONS.contains(&ext) {
                continue;
            }
        } else {
            continue;
        }
        // Test the path relative to the walk root for ignore matching.
        let rel = path.strip_prefix(root).unwrap_or(path);
        if ignore.is_match(rel) {
            continue;
        }
        out.push(path.to_path_buf());
    }
}

fn build_ignore_set(patterns: &[String]) -> anyhow::Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat)
            .map_err(|e| anyhow::anyhow!("invalid ignore pattern {:?}: {}", pat, e))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| anyhow::anyhow!("could not build ignore set: {}", e))
}

fn load_config(explicit: Option<&Path>) -> anyhow::Result<Config> {
    if let Some(path) = explicit {
        return Ok(Config::load(path)?);
    }
    let default_path = Path::new("funky.toml");
    if default_path.exists() {
        return Ok(Config::load(default_path)?);
    }
    Ok(Config::default())
}

fn read_source(path: &Path) -> anyhow::Result<String> {
    if path == Path::new("-") {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| anyhow::anyhow!("could not read stdin: {}", e))?;
        return Ok(s);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("could not read {}: {}", path.display(), e))?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{}: not valid UTF-8", path.display()))
}
