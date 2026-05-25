#@ run-status: 0

let co2miri = ($env.CO2_BIN_DIR | path join "co2miri")

let host_wrapper = (do { with-env { MIRI_BE_RUSTC: "host" } { ^$co2miri --version } } | complete)
if $host_wrapper.exit_code != 0 or ($host_wrapper.stdout | str contains "rustc") == false {
    print $"co2miri host-wrapper forwarding failed: ($host_wrapper | to json -r)"
    exit 1
}

let target_wrapper = (do { with-env { MIRI_BE_RUSTC: "target" } { ^$co2miri --error-format json --version } } | complete)
if $target_wrapper.exit_code != 0 or ($target_wrapper.stdout | str contains "rustc") == false {
    print $"co2miri target-wrapper forwarding failed: ($target_wrapper | to json -r)"
    exit 1
}

"#![language(co2)]\n" | save -f target_host.rs
"fn main() {}\n" | save -f target_host.co2

let target_compile = (
    do {
        with-env { MIRI_BE_RUSTC: "target" } {
            ^$co2miri target_host.rs --crate-name probe --crate-type bin --edition 2024 -o target-probe
        }
    } | complete
)
if $target_compile.exit_code != 0 or ("target-probe" | path exists) == false {
    print $"co2miri target compile path failed: ($target_compile | to json -r)"
    exit 1
}

"#![language(co2)]\n" | save -f target_fail.rs
"fn main() {\n    missing();\n}\n" | save -f target_fail.co2

let target_compile_fail = (
    do {
        with-env { MIRI_BE_RUSTC: "target" } {
            ^$co2miri target_fail.rs --crate-name probe_fail --crate-type bin --edition 2024 -o target-fail
        }
    } | complete
)
if $target_compile_fail.exit_code != 5 or ($target_compile_fail.stderr | str contains "Unresolved name missing") == false {
    print $"co2miri target diagnostic-abort path failed: ($target_compile_fail | to json -r)"
    exit 1
}

let interpreter_wrapper = (do { ^$co2miri --version -- ignored-program-arg } | complete)
if $interpreter_wrapper.exit_code != 0 or ($interpreter_wrapper.stdout | str contains "rustc") == false {
    print $"co2miri interpreter forwarding failed: ($interpreter_wrapper | to json -r)"
    exit 1
}
if ($interpreter_wrapper.stderr | str contains "multiple input filenames provided") {
    print $"co2miri treated program args as rustc inputs: ($interpreter_wrapper.stderr)"
    exit 1
}
