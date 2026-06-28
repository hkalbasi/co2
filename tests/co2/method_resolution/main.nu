#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let expected_dir = $env.CO2_TEST_SOURCE_DIR
let lib_rlib = ($test_dir | path join "libsupport_lib.rlib")
let app = ($test_dir | path join "app")

let compile_lib = (do {
    ^co2rustc support_lib.rs --crate-type=lib --crate-name support_lib --edition=2024 -o $lib_rlib
} | complete)
if $compile_lib.exit_code != 0 {
    print $"support_lib compile failed: ($compile_lib.stderr)"
    exit 1
}

let compile_bin = (do {
    ^co2rustc --edition=2024 ($test_dir | path join "main.rs") -o $app --extern $"support_lib=($lib_rlib)"
} | complete)
if $compile_bin.exit_code != 0 {
    print $"main compile failed: ($compile_bin.stderr)"
    exit 2
}

let run = (do { ^$app } | complete)
if $run.exit_code != 0 {
    print $"app failed: ($run.stderr)"
    exit 3
}

let actual = ($run.stdout | str trim)
assert-snapshot "stdout" $actual ($expected_dir | path join "stdout.expected")

exit 0
