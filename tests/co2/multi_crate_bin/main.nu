#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let src = ($test_dir | path join "workspace")
let lib_rlib = ($test_dir | path join "libsupport_lib.rlib")
let lib2_rlib = ($test_dir | path join "libsupport_lib2.rlib")
let app = ($test_dir | path join "app")
let lib_shim = ($src | path join "support_lib" "src" "lib.rs")
let lib2_shim = ($src | path join "support_lib2" "src" "lib.rs")

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
    ^rustc --edition=2024 ($src | path join "app" "src" "main.rs") -o $app -L $test_dir --extern support_lib --extern support_lib2
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

print "plain rustc multi-crate OK"

# Same crates through cargo: plain Rust bin (app) calling the co2 lib
# (support_lib), which itself calls the plain Rust lib (support_lib2).
let ws = ($test_dir | path join "workspace")

cd $ws

let cargo_build = (do { ^co2cargo build } | complete)
if $cargo_build.exit_code != 0 {
    print $"co2cargo build failed: ($cargo_build.stderr)"
    exit 5
}

let cargo_run = (do { ^co2cargo run } | complete)
if $cargo_run.exit_code != 0 {
    print $"co2cargo run failed: ($cargo_run.stdout) ($cargo_run.stderr)"
    exit 6
}

print "cargo multi-crate OK"

let miri_run = (do { ^co2cargo miri run } | complete)
if $miri_run.exit_code != 0 {
    print $"co2cargo miri run failed: ($miri_run.stdout) ($miri_run.stderr)"
    exit 7
}

print "miri multi-crate OK"

exit 0
