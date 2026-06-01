//! Pragma directive handling.
//!
//! Handles #pragma once, pack, push_macro/pop_macro, weak,
//! redefine_extname, and GCC visibility directives.

use super::pipeline::Preprocessor;

impl Preprocessor {
    pub(super) fn handle_pragma(&mut self, rest: &str) -> Option<String> {
        let rest = rest.trim();
        if rest == "once" {
            // Mark the current file as "include once"
            if let Some(current_file) = self.include_stack.last() {
                self.pragma_once_files.insert(current_file.clone());
            }
            return None;
        }

        // Handle #pragma pack directives.
        if let Some(pack_content) = rest.strip_prefix("pack") {
            return self.handle_pragma_pack(pack_content.trim());
        }

        // Handle #pragma push_macro("name") / pop_macro("name")
        if let Some(push_content) = rest.strip_prefix("push_macro") {
            self.handle_pragma_push_macro(push_content.trim());
            return None;
        }
        if let Some(pop_content) = rest.strip_prefix("pop_macro") {
            self.handle_pragma_pop_macro(pop_content.trim());
            return None;
        }

        // Handle #pragma weak symbol [= alias] — silently ignored
        if rest.strip_prefix("weak").is_some() {
            return None;
        }

        // Handle #pragma redefine_extname old new — silently ignored
        if rest.strip_prefix("redefine_extname").is_some() {
            return None;
        }

        // Handle #pragma GCC visibility push(hidden|default|protected|internal) / pop.
        if let Some(gcc_content) = rest.strip_prefix("GCC") {
            let gcc_content = gcc_content.trim();
            if let Some(vis_content) = gcc_content.strip_prefix("visibility") {
                return self.handle_pragma_gcc_visibility(vis_content.trim());
            }
        }

        // Other pragmas (GCC, diagnostic, etc.) are silently ignored
        None
    }

    /// Handle #pragma GCC visibility push(hidden|default|protected|internal) / pop.
    /// Emits synthetic tokens for the parser to track default visibility.
    fn handle_pragma_gcc_visibility(&mut self, content: &str) -> Option<String> {
        let content = content.trim();
        if content == "pop" {
            return Some("__ccc_visibility_pop ;\n".to_string());
        }
        if let Some(rest) = content.strip_prefix("push") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                let inner = rest.trim_start_matches('(').trim_end_matches(')').trim();
                match inner {
                    "hidden" | "default" | "protected" | "internal" => {
                        return Some(format!("__ccc_visibility_push_{inner} ;\n"));
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Handle #pragma push_macro("name") - save the current definition of macro.
    fn handle_pragma_push_macro(&mut self, content: &str) {
        if let Some(name) = Self::extract_pragma_macro_name(content) {
            let saved = self.macros.get(&name).cloned();
            self.macro_save_stack.entry(name).or_default().push(saved);
        }
    }

    /// Handle #pragma pop_macro("name") - restore the previously saved definition.
    fn handle_pragma_pop_macro(&mut self, content: &str) {
        if let Some(name) = Self::extract_pragma_macro_name(content)
            && let Some(stack) = self.macro_save_stack.get_mut(&name)
            && let Some(saved) = stack.pop()
        {
            match saved {
                Some(def) => self.macros.define(def),
                None => self.macros.undefine(&name),
            }
        }
    }

    /// Extract macro name from pragma argument like ("name").
    fn extract_pragma_macro_name(content: &str) -> Option<String> {
        let content = content.trim();
        if !content.starts_with('(') {
            return None;
        }
        let inner = content.trim_start_matches('(').trim_end_matches(')').trim();
        // Strip quotes
        let name = inner.trim_matches('"');
        if name.is_empty() {
            return None;
        }
        Some(name.to_string())
    }

    /// Handle #pragma pack directives and emit synthetic tokens for the parser.
    /// Supported forms:
    ///   #pragma pack(N)        - set alignment to N
    ///   #pragma pack()         - reset to default alignment
    ///   #pragma pack(push, N)  - push current and set to N
    ///   #pragma pack(push)     - push current (no change)
    ///   #pragma pack(pop)      - restore previous alignment
    fn handle_pragma_pack(&mut self, content: &str) -> Option<String> {
        let content = content.trim();
        // Must start with '('
        if !content.starts_with('(') {
            return None;
        }
        let inner = content.trim_start_matches('(').trim_end_matches(')').trim();

        if inner.is_empty() {
            // #pragma pack() - reset
            return Some("__ccc_pack_reset ;\n".to_string());
        }

        // Check for push/pop
        if inner == "pop" {
            return Some("__ccc_pack_pop ;\n".to_string());
        }

        if let Some(rest) = inner.strip_prefix("push") {
            let rest = rest.trim().trim_start_matches(',').trim();
            if rest.is_empty() {
                // #pragma pack(push) - push current alignment, don't change
                return Some("__ccc_pack_push_only ;\n".to_string());
            }
            // #pragma pack(push, N) - push current and set to N (0 means default)
            if let Ok(n) = rest.parse::<usize>() {
                return Some(format!("__ccc_pack_push_{n} ;\n"));
            }
            return None;
        }

        // #pragma pack(N) - set alignment
        if let Ok(n) = inner.parse::<usize>() {
            return Some(format!("__ccc_pack_set_{n} ;\n"));
        }

        None
    }
}
