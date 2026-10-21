use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(crate) enum LinkOutputKind {
    Executable,
    SharedLib,
}

pub(crate) struct DepFileArgs {
    pub(crate) generate: bool,
    pub(crate) exclude_system: bool,
    pub(crate) phony: bool,
    pub(crate) output: Option<PathBuf>,
    pub(crate) target: Option<String>,
    pub(crate) target_quoted: bool,
}

pub(crate) struct CcArgs {
    pub(crate) emit_obj_only: bool,
    pub(crate) emit_asm_only: bool,
    pub(crate) emit_llvm: bool,
    pub(crate) emit_preprocess_only: bool,
    pub(crate) link_kind: LinkOutputKind,
    pub(crate) pic: bool,
    pub(crate) time_report: bool,
    pub(crate) inputs: Vec<PathBuf>,
    pub(crate) output: Option<PathBuf>,
    pub(crate) opt_level: Option<String>,
    pub(crate) debuginfo: Option<u8>,
    pub(crate) cpp_args: Vec<String>,
    pub(crate) linker_args: Vec<String>,
    pub(crate) asm_flavor: Option<String>,
    pub(crate) dep_args: DepFileArgs,
    /// `-C target-cpu=` derived from `-march=<cpu>` (`native` included).
    pub(crate) target_cpu: Option<String>,
    /// `+feature`/`-feature` entries derived from `-m<feat>`/`-mno-<feat>`.
    pub(crate) target_features: Vec<String>,
    /// Original `-march=`/`-mtune=`/`-m<feat>` spellings, forwarded to
    /// sub-`co2cc` invocations and the assembler driver.
    pub(crate) arch_flags: Vec<String>,
    /// `-fsanitize=` entries, forwarded to rustc as `-Zsanitizer=`.
    pub(crate) sanitizers: Vec<String>,
    /// `-flto`/`-flto=full` (`-fno-lto` clears it), forwarded to rustc as
    /// `-C lto=fat` and to sub-`co2cc`/system-cc invocations as `-flto`.
    /// Only full LTO is supported: rustc's ThinLTO emits multiple objects
    /// per input, which co2cc's single-object pipeline cannot represent.
    pub(crate) lto: bool,
}

#[derive(Debug)]
pub(crate) enum ParseArgsError {
    MissingInputFile,
    MissingArgumentAfter(String),
    InvalidObjectEmissionInputs,
    InvalidArgument(String),
}

impl fmt::Display for ParseArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInputFile => {
                write!(f, "missing input file\nuse 'co2cc --help' for more info")
            }
            Self::MissingArgumentAfter(flag) => write!(f, "missing argument after {flag}"),
            Self::InvalidObjectEmissionInputs => {
                f.write_str("-c/-S mode expects exactly one C input file")
            }
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
        }
    }
}

pub(crate) fn parse_args(args: &[String]) -> Result<CcArgs, ParseArgsError> {
    if args.len() < 2 {
        return Err(ParseArgsError::MissingInputFile);
    }

    let mut emit_obj_only = false;
    let mut emit_asm_only = false;
    let mut emit_llvm = false;
    let mut emit_preprocess_only = false;
    let mut link_kind = LinkOutputKind::Executable;
    let mut pic = false;
    let mut time_report = false;
    let mut asm_flavor = None;
    let mut inputs = Vec::new();
    let mut output = None;
    let mut opt_level = None;
    let mut debuginfo = None;
    let mut cpp_args = Vec::new();
    let mut linker_args = Vec::new();
    let mut target_cpu = None;
    let mut target_features = Vec::new();
    let mut arch_flags: Vec<String> = Vec::new();
    let mut sanitizers: Vec<String> = Vec::new();
    let mut lto = false;
    let mut dep_args = DepFileArgs {
        generate: false,
        exclude_system: false,
        phony: false,
        output: None,
        target: None,
        target_quoted: false,
    };

    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-c" => emit_obj_only = true,
            "-S" => emit_asm_only = true,
            "-emit-llvm" => emit_llvm = true,
            "-E" => emit_preprocess_only = true,
            "-shared" => link_kind = LinkOutputKind::SharedLib,
            "-x" => {
                i += 1;
                args.get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter("-x".to_owned()))?;
            }
            "-o" => {
                i += 1;
                let out = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter("-o".to_owned()))?;
                output = Some(PathBuf::from(out));
            }
            "-I" | "-D" | "-U" | "-include" | "-isystem" | "-iquote" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
                cpp_args.push(flag);
                cpp_args.push(val.clone());
            }
            "-MD" => dep_args.generate = true,
            "-MMD" => {
                dep_args.generate = true;
                dep_args.exclude_system = true;
            }
            "-MP" => dep_args.phony = true,
            "-MF" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter("-MF".to_owned()))?;
                dep_args.output = Some(PathBuf::from(val));
            }
            "-MT" | "-MQ" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
                dep_args.target = Some(val.clone());
                dep_args.target_quoted = flag == "-MQ";
            }
            "-nostdinc" | "-undef" => cpp_args.push(arg.clone()),
            "-l" | "-L" | "-Wl" | "-Xlinker" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
                linker_args.push(flag);
                linker_args.push(val.clone());
            }
            f if f.starts_with("-masm=") => {
                let flavor = f.strip_prefix("-masm=").unwrap().to_owned();
                match flavor.as_str() {
                    "att" | "intel" => asm_flavor = Some(flavor),
                    _ => {
                        return Err(ParseArgsError::InvalidArgument(format!(
                            "'-masm={flavor}'; valid values: att, intel"
                        )));
                    }
                }
            }
            f if f.starts_with("-fsanitize=") || f.starts_with("-fno-sanitize=") => {
                let (flag, list) = f
                    .split_once('=')
                    .ok_or_else(|| ParseArgsError::InvalidArgument(format!("'{f}'")))?;
                apply_sanitize_list(&mut sanitizers, flag, list)?;
            }
            f if f.starts_with("-march=") => {
                let cpu = f.strip_prefix("-march=").unwrap().to_owned();
                if cpu.is_empty() {
                    return Err(ParseArgsError::InvalidArgument(format!("'{f}'")));
                }
                target_cpu = Some(cpu);
                arch_flags.push(arg.clone());
            }
            f if f.starts_with("-mtune=") => {
                // Scheduling-only, so accepted for compatibility without codegen effect.
                arch_flags.push(arg.clone());
            }
            f if f.starts_with("-m") && !f.starts_with("-M") => {
                if let Some(mapped) = map_m_feature_flag(f) {
                    for feature in mapped {
                        push_target_feature(&mut target_features, feature);
                    }
                    arch_flags.push(arg.clone());
                }
                // Unknown -m flags (e.g. -m32/-m64) are silently ignored.
            }
            "-fPIC" | "-fpic" => pic = true,
            "-flto" => lto = true,
            f if f.starts_with("-flto=") => {
                let mode = f.strip_prefix("-flto=").unwrap();
                match mode {
                    "full" => lto = true,
                    _ => {
                        return Err(ParseArgsError::InvalidArgument(format!(
                            "'-flto={mode}'; only full LTO is supported ('-flto', '-flto=full')"
                        )));
                    }
                }
            }
            "-fno-lto" => lto = false,
            "-fsanitize" | "-fno-sanitize" => {
                let flag = arg.clone();
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| ParseArgsError::MissingArgumentAfter(flag.clone()))?;
                apply_sanitize_list(&mut sanitizers, &flag, val)?;
            }
            "-ftime-report" => time_report = true,
            "-g0" => debuginfo = Some(0),
            "-g1" => debuginfo = Some(1),
            "-g" | "-g2" | "-g3" => debuginfo = Some(2),
            "-O0" | "-Og" => opt_level = Some("0".to_owned()),
            "-O1" => opt_level = Some("1".to_owned()),
            "-O2" => opt_level = Some("2".to_owned()),
            "-O3" => opt_level = Some("3".to_owned()),
            "-Os" => opt_level = Some("s".to_owned()),
            "-Oz" => opt_level = Some("z".to_owned()),
            "-" => inputs.push(PathBuf::from(arg)),
            _ if arg.starts_with("-I")
                || arg.starts_with("-D")
                || arg.starts_with("-U")
                || arg.starts_with("-std=")
                || arg.starts_with("-isystem")
                || arg.starts_with("-iquote")
                || arg.starts_with("-include") =>
            {
                cpp_args.push(arg.clone());
            }
            _ if arg.starts_with("-l")
                || arg.starts_with("-L")
                || arg.starts_with("-Wl,")
                || arg == "-pthread" =>
            {
                linker_args.push(arg.clone());
            }
            _ if arg.starts_with('-') => {}
            _ => {
                let path = PathBuf::from(arg);
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some("c" | "o" | "s" | "S") => inputs.push(path),
                    _ => linker_args.push(arg.clone()),
                }
            }
        }
        i += 1;
    }

    if inputs.is_empty() && linker_args.is_empty() {
        return Err(ParseArgsError::MissingInputFile);
    }
    // Like clang, `-fsanitize=cfi` is only allowed with `-flto`; rustc
    // rejects it later with a backend error, so fail fast here.
    if sanitizers.iter().any(|s| s == "cfi") && !lto {
        return Err(ParseArgsError::InvalidArgument(
            "'-fsanitize=cfi' only allowed with '-flto'".to_owned(),
        ));
    }
    if (emit_obj_only || emit_asm_only || emit_preprocess_only)
        && inputs
            .iter()
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("c" | "s" | "S")
                ) || p.as_os_str() == "-"
            })
            .count()
            != 1
    {
        return Err(ParseArgsError::InvalidObjectEmissionInputs);
    }

    Ok(CcArgs {
        emit_obj_only,
        emit_asm_only,
        emit_llvm,
        emit_preprocess_only,
        link_kind,
        pic,
        time_report,
        inputs,
        output,
        opt_level,
        debuginfo,
        cpp_args,
        linker_args,
        asm_flavor,
        dep_args,
        target_cpu,
        target_features,
        arch_flags,
        sanitizers,
        lto,
    })
}

/// cpp_args plus the original `-march=`/`-m<feat>` spellings so the
/// preprocessor can derive predefined feature macros (`__AVX__`, ...),
/// plus the canonical `-fsanitize=` spelling for `__SANITIZE_*__` macros.
pub(crate) fn preprocess_args(args: &CcArgs, cpp_args: &[String]) -> Vec<String> {
    cpp_args
        .iter()
        .cloned()
        .chain(args.arch_flags.iter().cloned())
        .chain(
            (!args.sanitizers.is_empty())
                .then(|| format!("-fsanitize={}", args.sanitizers.join(","))),
        )
        .collect()
}

/// Map a GCC-style `-m<feature>`/`-mno-<feature>` flag to rustc
/// `-C target-feature=` entries (`+feat` to enable, `-feat` to disable).
/// Returns `None` for flags co2cc does not model (silently ignored).
fn map_m_feature_flag(flag: &str) -> Option<Vec<String>> {
    let body = flag.strip_prefix("-m")?;
    let (negated, name) = match body.strip_prefix("no-") {
        Some(rest) => (true, rest),
        None => (false, body),
    };
    // rustc target-feature names for the matching GCC -m spelling.
    // (`-msse4` enables both halves, like GCC.)
    let feats: &[&str] = match name {
        "avx" => &["avx"],
        "avx2" => &["avx2"],
        "fma" => &["fma"],
        "sse" => &["sse"],
        "sse2" => &["sse2"],
        "sse3" => &["sse3"],
        "ssse3" => &["ssse3"],
        "sse4" => &["sse4.1", "sse4.2"],
        "sse4.1" => &["sse4.1"],
        "sse4.2" => &["sse4.2"],
        "avx512f" => &["avx512f"],
        "avx512cd" => &["avx512cd"],
        "avx512dq" => &["avx512dq"],
        "avx512bw" => &["avx512bw"],
        "avx512vl" => &["avx512vl"],
        "avx512vbmi" => &["avx512vbmi"],
        "avx512vbmi2" => &["avx512vbmi2"],
        "bmi" => &["bmi1"],
        "bmi2" => &["bmi2"],
        "lzcnt" => &["lzcnt"],
        "abm" => &["lzcnt", "popcnt"],
        "popcnt" => &["popcnt"],
        "aes" => &["aes"],
        "pclmul" => &["pclmulqdq"],
        "rdrnd" => &["rdrand"],
        "rdseed" => &["rdseed"],
        "sha" => &["sha"],
        "adx" => &["adx"],
        "movbe" => &["movbe"],
        "fxsr" => &["fxsr"],
        "xsave" => &["xsave"],
        "xsaveopt" => &["xsaveopt"],
        "fsgsbase" => &["fsgsbase"],
        "cx16" => &["cmpxchg16b"],
        _ => return None,
    };
    let sign = if negated { '-' } else { '+' };
    Some(feats.iter().map(|feat| format!("{sign}{feat}")).collect())
}

/// Record a `+feat`/`-feat` entry, with later flags winning over earlier
/// ones for the same feature (matching GCC last-flag-wins).
fn push_target_feature(features: &mut Vec<String>, feature: String) {
    let name = &feature[1..];
    features.retain(|existing| &existing[1..] != name);
    features.push(feature);
}

/// Record (`-fsanitize=`) or remove (`-fno-sanitize=`) a comma-separated
/// sanitizer list, passed through verbatim to rustc `-Zsanitizer=`.
/// Names are validated against rustc's accepted set so typos and
/// unsupported sanitizers (e.g. `undefined`) fail fast instead of
/// surfacing as cryptic rustc errors.
fn apply_sanitize_list(
    sanitizers: &mut Vec<String>,
    flag: &str,
    list: &str,
) -> Result<(), ParseArgsError> {
    let negated = flag == "-fno-sanitize";
    let mut any = false;
    for name in list.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        any = true;
        if !VALID_SANITIZERS.contains(&name) {
            return Err(ParseArgsError::InvalidArgument(format!(
                "'{flag}={name}'; valid values: {}",
                VALID_SANITIZERS.join(", ")
            )));
        }
        if negated {
            sanitizers.retain(|existing| existing != name);
        } else if !sanitizers.iter().any(|existing| existing == name) {
            sanitizers.push(name.to_owned());
        }
    }
    if !any {
        return Err(ParseArgsError::InvalidArgument(format!("'{flag}={list}'")));
    }
    Ok(())
}

/// Sanitizer names accepted by `-fsanitize=` (mirrors rustc `-Zsanitizer=`).
const VALID_SANITIZERS: &[&str] = &[
    "address",
    "cfi",
    "dataflow",
    "hwaddress",
    "kcfi",
    "kernel-address",
    "kernel-hwaddress",
    "leak",
    "memory",
    "memtag",
    "safestack",
    "shadow-call-stack",
    "thread",
    "realtime",
];
