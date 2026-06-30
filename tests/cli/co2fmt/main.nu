#@ run-status: 0

let co2fmt = ($env.CO2_BIN_DIR | path join "co2fmt")

# ---- help ----
let help = (do { ^$co2fmt --help } | complete)
if $help.exit_code != 0 {
    print $"FAIL: --help exit code expected 0, got ($help.exit_code)"
    exit 1
}
if ($help.stdout | str contains "C/C++ formatter") == false {
    print "FAIL: --help missing expected description"
    exit 1
}

# ---- version ----
let ver = (do { with-env { CO2_VERSION: "test" } { ^$co2fmt --version } } | complete)
if $ver.exit_code != 0 {
    print $"FAIL: --version exit code expected 0, got ($ver.exit_code)"
    exit 1
}
if ($ver.stdout | str trim) != "co2fmt test" {
    print $"FAIL: --version expected 'co2fmt test', got '($ver.stdout | str trim)'"
    exit 1
}

# --- test data ----
let formatted = "fn foo() {\n    int x = 2;\n}\n"
let ugly = "fn foo()\n{\nint x = 2;\n}\n"

# ---- --check pass on already-formatted file ----
$formatted | save -f formatted.co2
let check_pass = (do { ^$co2fmt --check formatted.co2 } | complete)
if $check_pass.exit_code != 0 {
    print $"FAIL: --check on formatted file expected 0, got ($check_pass.exit_code): ($check_pass.stderr)"
    exit 1
}

# ---- --check fail on ugly file ----
$ugly | save -f ugly.co2
let check_fail = (do { ^$co2fmt --check ugly.co2 } | complete)
if $check_fail.exit_code != 1 {
    print $"FAIL: --check on ugly file expected 1, got ($check_fail.exit_code)"
    exit 1
}

# ---- stdout formatting ----
let stdout_out = (do { ^$co2fmt ugly.co2 } | complete)
if $stdout_out.exit_code != 0 {
    print $"FAIL: fmt ugly.co2 expected 0, got ($stdout_out.exit_code): ($stdout_out.stderr)"
    exit 1
}
if ($stdout_out.stdout | str trim) != ($formatted | str trim) {
    print $"FAIL: formatting stdout mismatch"
    exit 1
}

# ---- stdin formatting (no args) ----
let stdin_out = (do { $ugly | ^$co2fmt } | complete)
if $stdin_out.exit_code != 0 {
    print $"FAIL: stdin fmt expected 0, got ($stdin_out.exit_code): ($stdin_out.stderr)"
    exit 1
}
if ($stdin_out.stdout | str trim) != ($formatted | str trim) {
    print $"FAIL: stdin formatting mismatch"
    exit 1
}

# ---- --emit files (in-place) ----
$ugly | save -f emit_test.co2
let emit_files = (do { ^$co2fmt --emit files emit_test.co2 } | complete)
if $emit_files.exit_code != 0 {
    print $"FAIL: --emit files expected 0, got ($emit_files.exit_code): ($emit_files.stderr)"
    exit 1
}
let emit_content = (open emit_test.co2 | str trim)
if $emit_content != ($formatted | str trim) {
    print $"FAIL: --emit files did not format in place"
    exit 1
}

# ---- --check --files-with-diff ----
$ugly | save -f diff_test.co2
let diff_out = (do { ^$co2fmt --check --files-with-diff diff_test.co2 } | complete)
if $diff_out.exit_code != 1 {
    print $"FAIL: --check --files-with-diff expected 1, got ($diff_out.exit_code)"
    exit 1
}
if ($diff_out.stdout | str trim) != "diff_test.co2" {
    print $"FAIL: --check --files-with-diff expected filename, got: ($diff_out.stdout)"
    exit 1
}

# ---- --emit files + stdin (should still go to stdout) ----
let stdin_emit = (do { $ugly | ^$co2fmt --emit files } | complete)
if $stdin_emit.exit_code != 0 {
    print $"FAIL: stdin+--emit files expected 0, got ($stdin_emit.exit_code): ($stdin_emit.stderr)"
    exit 1
}
if ($stdin_emit.stdout | str trim) != ($formatted | str trim) {
    print $"FAIL: stdin+--emit files formatting mismatch"
    exit 1
}

# ---- --check on stdin (pass) ----
let check_stdin_pass = (do { $formatted | ^$co2fmt --check } | complete)
if $check_stdin_pass.exit_code != 0 {
    print $"FAIL: --check on formatted stdin expected 0, got ($check_stdin_pass.exit_code)"
    exit 1
}

# ---- --check on stdin (fail) ----
let check_stdin_fail = (do { $ugly | ^$co2fmt --check } | complete)
if $check_stdin_fail.exit_code != 1 {
    print $"FAIL: --check on ugly stdin expected 1, got ($check_stdin_fail.exit_code)"
    exit 1
}

# ---- --edition, --color, --config, --verbose, --quiet are accepted but ignored ----
let ignored = (do { ^$co2fmt --edition c17 --color never --config indent_width=4 --verbose --quiet formatted.co2 } | complete)
if $ignored.exit_code != 0 {
    print $"FAIL: ignored flags expected 0, got ($ignored.exit_code): ($ignored.stderr)"
    exit 1
}

print "ALL CHECKS PASSED"
