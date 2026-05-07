# Test: co2cargo miri run forwards args after `--` as program argv.
#
# Regression test for cases like:
#   co2cargo miri run -- -e "console.log(2)"
# where `-e` must not be treated as another rustc input filename.

let test_dir = $env.CO2_TEST_DIR

let miri_check = (do { ^rustup which cargo-miri } | complete)
if $miri_check.exit_code != 0 {
    print "skip: cargo-miri not available (install miri: rustup component add miri)"
    exit 0
}

let project = ($test_dir | path join "miri_run_args_project")
mkdir $project

"[package]
name = \"miri_run_args_project\"
version = \"0.1.0\"
edition = \"2024\"
" | save ($project | path join "Cargo.toml")

mkdir ($project | path join "src")

"#![language(co2)]
" | save ($project | path join "src" "main.rs")

"fn main() {}
" | save ($project | path join "src" "main.co2")

cd $project
let result = (do { ^co2cargo miri run -- -e "console.log(2)" } | complete)

if $result.exit_code != 0 {
    print "FAIL: co2cargo miri run with program args failed"
    print $"stdout: ($result.stdout)"
    print $"stderr: ($result.stderr)"
    exit 1
}

if ($result.stderr | str contains "multiple input filenames provided") {
    print "FAIL: program args were interpreted as rustc input filenames"
    print $"stderr: ($result.stderr)"
    exit 1
}

print "co2cargo miri run arg forwarding test passed"
exit 0
