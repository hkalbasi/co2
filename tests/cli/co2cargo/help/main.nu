#@ run-status: 0

use ./snapshot-utils.nu *

let co2cargo = ($env.CO2_BIN_DIR | path join "co2cargo")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cargo" "help")

# ---- co2cargo --help ----
let help = (do { ^$co2cargo --help } | complete)
if $help.exit_code != 0 {
    print $"FAIL: co2cargo --help exit code expected 0, got ($help.exit_code)"
    exit 1
}
if ($help.stderr | str trim) != "" {
    print "FAIL: co2cargo --help expected empty stderr"
    exit 1
}
assert-snapshot "co2cargo --help" $help.stdout ($expected_dir | path join "co2cargo--help.stdout.snapshot")

# ---- co2cargo --version ----
let version = (do { with-env { CO2_VERSION: "test" } { ^$co2cargo --version } } | complete)
if $version.exit_code != 0 {
    print $"FAIL: co2cargo --version exit code expected 0, got ($version.exit_code)"
    exit 1
}
if ($version.stderr | str trim) != "" {
    print "FAIL: co2cargo --version expected empty stderr"
    exit 1
}
assert-snapshot "co2cargo --version" $version.stdout ($expected_dir | path join "co2cargo--version.stdout.snapshot")

# ---- co2cargo run --help ----
let run_help = (do { ^$co2cargo run --help } | complete)
if $run_help.exit_code != 0 {
    print $"FAIL: co2cargo run --help exit code expected 0, got ($run_help.exit_code)"
    exit 1
}
if ($run_help.stderr | str trim) != "" {
    print "FAIL: co2cargo run --help expected empty stderr"
    exit 1
}
assert-snapshot "co2cargo run --help" $run_help.stdout ($expected_dir | path join "co2cargo-run--help.stdout.snapshot")
