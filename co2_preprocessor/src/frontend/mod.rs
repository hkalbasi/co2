pub mod preprocessor;

pub mod sema {
    pub mod builtins {
        pub fn is_builtin(_name: &str) -> bool {
            false
        }
    }
}