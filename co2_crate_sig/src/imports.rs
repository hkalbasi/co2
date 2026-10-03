//! Rust-compatible `use` import priority.
//!
//! Policy (mirrors rustc):
//! - An explicit single import shadows glob imports regardless of order.
//! - A name provided by multiple distinct glob sources with no other binding
//!   is ambiguous and only errors when used (E0659).
//! - Two explicit imports of the same name conflict at import time (E0252).
//! - A local item already binding a name wins silently over any import.
//!
//! The tracker owns only this policy state; the module tree stays in
//! [`crate::resolver`]. [`ImportTracker::ambiguous_in_scope`] walks scopes
//! innermost-out like name resolution, so an inner binding clears an outer
//! ambiguity.

use rustc_data_structures::fx::{FxHashMap, FxHashSet};

use super::resolver::ModuleData;
use rustc_public_generative::DependencyInfo;

/// Decision for an explicit single import of `alias` into a module.
pub(crate) enum SingleDecision {
    /// Insert it, overwriting any glob entry.
    Proceed,
    /// A local item already binds the name; keep it and ignore the import.
    KeepLocal,
    /// A previous explicit import binds the name; carries the E0252 message.
    Duplicate(String),
}

#[derive(Debug, Default)]
pub(crate) struct ImportTracker {
    /// Explicit single imports, by module key then alias.
    singles: FxHashMap<String, FxHashSet<String>>,
    /// Which glob source first brought each name in, by module key then name.
    /// Names bound by a local item or single import are never recorded here.
    glob_sources: FxHashMap<String, FxHashMap<String, String>>,
    /// Names from multiple distinct glob sources with no other binding.
    ambiguous: FxHashMap<String, FxHashSet<String>>,
}

impl ImportTracker {
    pub(crate) fn check_single(&self, mod_key: &str, alias: &str, bound: bool) -> SingleDecision {
        if self
            .singles
            .get(mod_key)
            .is_some_and(|names| names.contains(alias))
        {
            return SingleDecision::Duplicate(format!(
                "the name `{alias}` is defined multiple times"
            ));
        }
        if bound
            && !self
                .glob_sources
                .get(mod_key)
                .is_some_and(|names| names.contains_key(alias))
        {
            return SingleDecision::KeepLocal;
        }
        SingleDecision::Proceed
    }

    /// Record an explicit import, resolving any glob ambiguity for the name.
    pub(crate) fn commit_single(&mut self, mod_key: &str, alias: &str) {
        self.singles
            .entry(mod_key.to_owned())
            .or_default()
            .insert(alias.to_owned());
        if let Some(names) = self.glob_sources.get_mut(mod_key) {
            names.remove(alias);
        }
        if let Some(names) = self.ambiguous.get_mut(mod_key) {
            names.remove(alias);
        }
    }

    /// Record one name from a glob import. Returns whether the caller should
    /// insert it into the module (false when shadowed by a single or local).
    /// A repeated identical glob is harmless; a distinct source marks the
    /// name ambiguous.
    pub(crate) fn note_glob(
        &mut self,
        mod_key: &str,
        name: &str,
        source: &str,
        bound: bool,
    ) -> bool {
        if self
            .singles
            .get(mod_key)
            .is_some_and(|names| names.contains(name))
        {
            return false;
        }
        match self
            .glob_sources
            .get(mod_key)
            .and_then(|names| names.get(name))
        {
            None if bound => return false,
            Some(prev) if *prev != source => {
                self.ambiguous
                    .entry(mod_key.to_owned())
                    .or_default()
                    .insert(name.to_owned());
            }
            Some(_) => {}
            None => {
                self.glob_sources
                    .entry(mod_key.to_owned())
                    .or_default()
                    .insert(name.to_owned(), source.to_owned());
            }
        }
        true
    }

    pub(crate) fn is_ambiguous(&self, mod_key: &str, name: &str) -> bool {
        self.ambiguous
            .get(mod_key)
            .is_some_and(|names| names.contains(name))
    }

    /// Whether `name` is unusable in `module_path` due to conflicting glob
    /// imports. Stops at the innermost scope binding the name.
    pub(crate) fn ambiguous_in_scope(
        &self,
        root: &mut ModuleData,
        module_path: &[String],
        name: &str,
        info: &DependencyInfo<'_>,
    ) -> bool {
        for prefix_len in (0..=module_path.len()).rev() {
            if !scope_has_name(root, &module_path[..prefix_len], name, info) {
                continue;
            }
            return self.is_ambiguous(&module_path[..prefix_len].join("::"), name);
        }
        false
    }

    /// Whether the last segment of an anchored (`crate::`/`super::`) path is
    /// an ambiguous glob import in the module the anchor points at.
    pub(crate) fn ambiguous_at_anchor(
        &self,
        module_path: &[String],
        anchor: &str,
        supers: usize,
        name: &str,
    ) -> bool {
        let key = match anchor {
            "crate" => String::new(),
            "super" if supers <= module_path.len() => {
                module_path[..module_path.len() - supers].join("::")
            }
            _ => return false,
        };
        self.is_ambiguous(&key, name)
    }
}

fn scope_has_name(
    node: &mut ModuleData,
    segments: &[String],
    name: &str,
    info: &DependencyInfo<'_>,
) -> bool {
    let mut current = node;
    for segment in segments {
        let Some(next) = current.as_content_mut().items.get_mut(segment) else {
            return false;
        };
        current = next;
    }
    current.resolve_path([name].into_iter(), info).is_some()
}
