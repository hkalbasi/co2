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

exit 0
