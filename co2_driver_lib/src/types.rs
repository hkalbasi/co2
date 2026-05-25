use rustc_public_generative as rustc_gen;

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct CompileMode {
    pub no_main: bool,
    pub function_abi: rustc_gen::FunctionAbi,
    pub function_no_mangle: bool,
    pub function_is_unsafe: bool,
    pub test: bool,
}

impl CompileMode {
    pub const RUST: Self = Self {
        no_main: false,
        function_abi: rustc_gen::FunctionAbi::Rust,
        function_no_mangle: false,
        function_is_unsafe: false,
        test: false,
    };

    pub const RUST_TEST: Self = Self {
        test: true,
        ..Self::RUST
    };

    pub const C: Self = Self {
        no_main: true,
        function_abi: rustc_gen::FunctionAbi::C,
        function_no_mangle: true,
        function_is_unsafe: false,
        test: false,
    };
}
