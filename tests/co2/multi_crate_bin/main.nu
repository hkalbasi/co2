#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let lib_rlib = ($test_dir | path join "libsupport_lib.rlib")
let lib2_rlib = ($test_dir | path join "libsupport_lib2.rlib")
let app = ($test_dir | path join "app")
let lib_shim = ($test_dir | path join "support_lib.rs")
let lib2_shim = ($test_dir | path join "support_lib2.rs")

'#![language(co2)]' | save -f $lib_shim

let compile_lib2 = (do {
    ^co2rustc $lib2_shim --crate-type=lib --crate-name support_lib2 --edition=2024 -o $lib2_rlib
} | complete)
if $compile_lib2.exit_code != 0 {
    print $"support_lib2 compile failed: ($compile_lib2.stderr)"
    exit 2
}

let compile_lib = (do {
    ^co2rustc $lib_shim --crate-type=lib --crate-name support_lib --edition=2024 -o $lib_rlib --extern $"support_lib2=($lib2_rlib)"
} | complete)
if $compile_lib.exit_code != 0 {
    print $"support_lib compile failed: ($compile_lib.stderr)"
    exit 1
}

let compile_bin = (do {
    ^rustc --edition=2024 ($test_dir | path join "main.rs") -o $app -L $test_dir --extern support_lib --extern support_lib2
} | complete)
if $compile_bin.exit_code != 0 {
    print $"main compile failed: ($compile_bin.stderr)"
    exit 3
}

let run = (do { ^$app } | complete)
if $run.exit_code != 0 {
    print $"app failed: ($run.stderr)"
    exit 4
}

exit 0