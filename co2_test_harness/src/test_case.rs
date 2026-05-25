use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    C,
    Co2,
    Rust,
}

impl Mode {
    pub fn from_directive(s: &str) -> Result<Self> {
        match s {
            "c" => Ok(Self::C),
            "co2" => Ok(Self::Co2),
            "rust" => Ok(Self::Rust),
            _ => bail!("unknown mode: {s}"),
        }
    }
}

pub struct TestCase {
    pub path: PathBuf,
    pub kind: TestKind,
    pub directives: HashMap<String, Vec<String>>,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestKind {
    File,
    NuDir,
}

pub enum TestOutcome {
    Pass,
    Skip(String),
}

pub fn collect_tests(root: &Path, filter: Option<&str>) -> Result<Vec<TestCase>> {
    let mut out = Vec::new();
    let dir = root.join("tests");
    if !dir.exists() {
        return Ok(out);
    }
    collect_tests_inner(root, &dir, filter, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn collect_tests_inner(
    root: &Path,
    dir: &Path,
    filter: Option<&str>,
    out: &mut Vec<TestCase>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let main_nu = path.join("main.nu");
            if main_nu.exists() {
                if let Some(pattern) = filter
                    && !path_matches(root, &path, pattern)
                {
                    continue;
                }
                out.push(TestCase {
                    directives: parse_directives(&main_nu)?,
                    path,
                    kind: TestKind::NuDir,
                    source: String::new(),
                });
                continue;
            }
            collect_tests_inner(root, &path, filter, out)?;
            continue;
        }

        let Some(ext) = path.extension().and_then(OsStr::to_str) else {
            continue;
        };
        if ext != "c" && ext != "co2" && ext != "rs" {
            continue;
        }
        if let Some(pattern) = filter
            && !path_matches(root, &path, pattern)
        {
            continue;
        }

        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read test source {}", path.display()))?;
        let directives = parse_directives(&path)?;
        if !directives.contains_key("mode") {
            continue;
        }
        out.push(TestCase {
            directives,
            path,
            kind: TestKind::File,
            source,
        });
    }
    Ok(())
}

fn path_matches(root: &Path, path: &Path, filter: &str) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    glob_matches(filter, &relative)
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    fn inner(pattern: &[u8], text: &[u8]) -> bool {
        if pattern.is_empty() {
            return text.is_empty();
        }

        match pattern[0] {
            b'*' if pattern.get(1) == Some(&b'*') => {
                inner(&pattern[2..], text) || (!text.is_empty() && inner(pattern, &text[1..]))
            }
            b'*' => {
                inner(&pattern[1..], text)
                    || (!text.is_empty() && text[0] != b'/' && inner(pattern, &text[1..]))
            }
            b'?' => !text.is_empty() && text[0] != b'/' && inner(&pattern[1..], &text[1..]),
            b => !text.is_empty() && b == text[0] && inner(&pattern[1..], &text[1..]),
        }
    }

    inner(pattern.as_bytes(), text.as_bytes())
}

pub fn parse_directives(path: &Path) -> Result<HashMap<String, Vec<String>>> {
    let src = fs::read_to_string(path)
        .with_context(|| format!("failed to read test source {}", path.display()))?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    for line in src.lines() {
        let line = line.trim_start();
        let body = if let Some(body) = line.strip_prefix("//@") {
            body.trim()
        } else if let Some(body) = line.strip_prefix("#@") {
            body.trim()
        } else {
            continue;
        };
        let (key, value) = if let Some((k, v)) = body.split_once(':') {
            (k.trim().to_owned(), v.trim().to_owned())
        } else {
            (body.to_owned(), String::new())
        };
        map.entry(key).or_default().push(value);
    }

    Ok(map)
}

pub fn directive_text(test: &TestCase, key: &str) -> Option<String> {
    test.directives.get(key).and_then(|v| v.first().cloned())
}

pub fn directive_i32(test: &TestCase, key: &str) -> Result<Option<i32>> {
    let Some(raw) = directive_text(test, key) else {
        return Ok(None);
    };
    let parsed = raw
        .parse::<i32>()
        .with_context(|| format!("directive `{key}` must be an integer, got `{raw}`"))?;
    Ok(Some(parsed))
}

pub fn directive_args(test: &TestCase, key: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for raw in test.directives.get(key).cloned().unwrap_or_default() {
        let split = shlex::split(&raw)
            .with_context(|| format!("failed to parse `{key}` args from `{raw}`"))?;
        out.extend(split);
    }
    Ok(out)
}
