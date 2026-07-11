#@ run-status: 0

use ./snapshot-utils.nu *

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")
let co2rustc = ($env.CO2_BIN_DIR | path join "co2rustc")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "link_error")
let test_dir = $env.CO2_TEST_DIR

# ---- co2cc link error ----
let result_co2cc = (do { ^$co2cc test.c -o test_out } | complete)
let actual_co2cc = (
    $result_co2cc.stderr
    | str replace -a -r '\x1b\[[0-9;]*[a-zA-Z]' ''
    | str replace -a $test_dir "[TEMP]"
    | str replace -a -r '/tmp/\.co2cc-\d+-\d+' "[TEMP]"
    | str replace -a -r 'rustc[A-Za-z0-9]{6}' "[RUSTC_TEMP]"
    | str trim
)
assert-snapshot "co2cc stderr" $actual_co2cc ($expected_dir | path join "co2cc.stderr.snapshot")

# ---- co2rustc link error ----
let result_co2rustc = (do { ^$co2rustc test.co2 -o test_out2 } | complete)
let actual_co2rustc = (
    $result_co2rustc.stderr
    | str replace -a -r '\x1b\[[0-9;]*[a-zA-Z]' ''
    | str replace -a $test_dir "[TEMP]"
    | str replace -a -r 'rustc[A-Za-z0-9]{6}' "[RUSTC_TEMP]"
    | str trim
)
assert-snapshot "co2rustc stderr" $actual_co2rustc ($expected_dir | path join "co2rustc.stderr.snapshot")

# ---- co2cc shared library: default keeps undefined symbols as holes (like gcc) ----
let result_shared_default = (do { ^$co2cc -shared -fPIC test_shared.c -o test_shared.so } | complete)
if $result_shared_default.exit_code != 0 {
    print $"shared library default build failed: ($result_shared_default.stderr)"
    exit 1
}
if not ("test_shared.so" | path exists) {
    print "shared library default build did not produce test_shared.so"
    exit 1
}
# `foo` must remain an undefined (hole) symbol in the shared library.
let hole_syms = (
    ^nm -D test_shared.so
    | lines
    | where ($it | str trim | str starts-with "U ")
    | where ($it | str contains "foo")
)
if ($hole_syms | is-empty) {
    print "expected `foo` to be left as an undefined symbol (hole) in the shared library"
    exit 1
}

# ---- co2cc shared library link error: --no-undefined forces the error ----
let result_shared = (do { ^$co2cc -shared -fPIC -Wl,--no-undefined test_shared.c -o test_shared.so } | complete)
let actual_shared = (
    $result_shared.stderr
    | str replace -a -r '\x1b\[[0-9;]*[a-zA-Z]' ''
    | str replace -a $test_dir "[TEMP]"
    | str replace -a -r '/tmp/\.co2cc-\d+-\d+' "[TEMP]"
    | str replace -a -r 'rustc[A-Za-z0-9]{6}' "[RUSTC_TEMP]"
    | str trim
)
assert-snapshot "co2cc shared lib stderr" $actual_shared ($expected_dir | path join "co2cc_shared.stderr.snapshot")

exit 0
