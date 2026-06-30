#@ run-status: 0

use ./snapshot-utils.nu *

let co2rustc = ($env.CO2_BIN_DIR | path join "co2rustc")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2rustc" "error_paths")

# ---- no args ----
let no_args = (do { ^$co2rustc } | complete)
if $no_args.exit_code != 0 {
    print $"FAIL: co2rustc no args exit code expected 0, got ($no_args.exit_code)"
    exit 1
}
if ($no_args.stderr | str trim) != "" {
    print "FAIL: co2rustc no args expected empty stderr"
    exit 1
}
assert-snapshot "no_args" $no_args.stdout ($expected_dir | path join "no_args.stdout.snapshot")

# ---- --version ----
let version = (do { with-env { CO2_VERSION: "test" } { ^$co2rustc --version } } | complete)
if $version.exit_code != 0 {
    print $"FAIL: co2rustc --version exit code expected 0, got ($version.exit_code)"
    exit 1
}
if ($version.stderr | str trim) != "" {
    print "FAIL: co2rustc --version expected empty stderr"
    exit 1
}
assert-snapshot "version" $version.stdout ($expected_dir | path join "version.stdout.snapshot")

# ---- -h and --help ----
let help_h = (do { ^$co2rustc -h } | complete)
if $help_h.exit_code != 0 {
    print $"FAIL: co2rustc -h exit code expected 0, got ($help_h.exit_code)"
    exit 1
}
if ($help_h.stderr | str trim) != "" {
    print "FAIL: co2rustc -h expected empty stderr"
    exit 1
}
assert-snapshot "help_h" $help_h.stdout ($expected_dir | path join "help_h.stdout.snapshot")

let help_help = (do { ^$co2rustc --help } | complete)
if $help_help.exit_code != 0 {
    print $"FAIL: co2rustc --help exit code expected 0, got ($help_help.exit_code)"
    exit 1
}
if ($help_help.stderr | str trim) != "" {
    print "FAIL: co2rustc --help expected empty stderr"
    exit 1
}
assert-snapshot "help_help" $help_help.stdout ($expected_dir | path join "help_help.stdout.snapshot")

# ---- missing -o argument ----
let missing_o = (do { ^$co2rustc -o } | complete)
if $missing_o.exit_code != 1 {
    print $"FAIL: co2rustc -o exit code expected 1, got ($missing_o.exit_code)"
    exit 1
}
if ($missing_o.stdout | str trim) != "" {
    print "FAIL: co2rustc -o expected empty stdout"
    exit 1
}
assert-snapshot "missing_o" $missing_o.stderr ($expected_dir | path join "missing_o.stderr.snapshot")

# ---- multiple input files ----
let multiple_inputs = (do { ^$co2rustc a.rs b.rs } | complete)
if $multiple_inputs.exit_code != 1 {
    print $"FAIL: co2rustc multiple inputs exit code expected 1, got ($multiple_inputs.exit_code)"
    exit 1
}
if ($multiple_inputs.stdout | str trim) != "" {
    print "FAIL: co2rustc multiple inputs expected empty stdout"
    exit 1
}
assert-snapshot "multiple_inputs" $multiple_inputs.stderr ($expected_dir | path join "multiple_inputs.stderr.snapshot")

# ---- non-existing .rs file ----
let non_existing_rs = (do { ^$co2rustc ./non-existing.rs } | complete)
if $non_existing_rs.exit_code != 1 {
    print $"FAIL: co2rustc ./non-existing.rs exit code expected 1, got ($non_existing_rs.exit_code)"
    exit 1
}
if ($non_existing_rs.stdout | str trim) != "" {
    print "FAIL: co2rustc ./non-existing.rs expected empty stdout"
    exit 1
}
assert-snapshot "non_existing_rs" $non_existing_rs.stderr ($expected_dir | path join "non_existing_rs.stderr.snapshot")

