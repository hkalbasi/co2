use std::path::PathBuf;

use itertools::Itertools;
use rustc_ast::{Attribute, MetaItemKind};
use rustc_driver::{Callbacks, Compilation};

fn is_language_co2(attr: &Attribute) -> bool {
    let meta = match attr.meta() {
        Some(meta) => meta,
        None => return false,
    };

    let path_segments = meta
        .path
        .segments
        .iter()
        .map(|s| s.ident.as_str())
        .collect::<Vec<_>>();

    if path_segments.as_slice() == ["language_co2"] {
        return true;
    }

    if path_segments.as_slice() == ["co2", "language"] {
        return true;
    }

    let is_language = meta
        .path
        .segments
        .iter()
        .map(|s| s.ident.as_str())
        .eq(std::iter::once("language"));

    if !is_language {
        return false;
    }

    match &meta.kind {
        MetaItemKind::List(items) => items.iter().any(|item| match item {
            rustc_ast::MetaItemInner::MetaItem(item) => {
                item.path.segments.iter().all(|s| s.ident.as_str() == "co2")
            }
            rustc_ast::MetaItemInner::Lit(_) => false,
        }),
        _ => false,
    }
}

struct DetectCallbacks {
    co2_file: Option<PathBuf>,
    enabled: bool,
}

impl DetectCallbacks {
    fn new() -> Self {
        Self {
            co2_file: None,
            enabled: false,
        }
    }
}

impl Callbacks for DetectCallbacks {
    fn after_crate_root_parsing(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        for attr in &krate.attrs {
            if is_language_co2(attr) {
                self.enabled = true;
                let files_lock = compiler.sess.source_map().files();
                let original_file = files_lock.iter().exactly_one().unwrap();

                let rustc_span::FileName::Real(original_file) = &original_file.name else {
                    panic!("File was not real");
                };

                let original_file = original_file.path(rustc_span::RemapPathScopeComponents::MACRO);
                let co2_file = original_file.with_extension("co2");
                drop(files_lock);
                self.co2_file = Some(co2_file);
                return Compilation::Stop;
            }
        }

        Compilation::Continue
    }
}

pub(crate) enum DetectResult {
    Continue(i32),
    Co2(PathBuf),
}

pub(crate) fn detect_co2(args: &[String]) -> DetectResult {
    let mut callbacks = DetectCallbacks::new();

    let exit_code =
        rustc_driver::catch_with_exit_code(|| rustc_driver::run_compiler(args, &mut callbacks));

    if !callbacks.enabled {
        return DetectResult::Continue(exit_code);
    }

    DetectResult::Co2(callbacks.co2_file.expect("co2 file missing"))
}
