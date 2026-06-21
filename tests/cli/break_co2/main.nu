#@ run-status: 0

use ./snapshot-utils.nu *

def normalize [s: string] {
    $s
    | str replace --all --regex '\(\d+\) panicked' '(PID) panicked'
    | str replace --all --regex 'panicked at .*co2_crate_sig[/\\]src[/\\]lowering\.rs:\d+:\d+' 'panicked at co2_crate_sig/src/lowering.rs:LINE:COL'
    | str replace --all --regex 'panicked at .*co2cc[/\\]src[/\\]lib\.rs:\d+:\d+' 'panicked at co2cc/src/lib.rs:LINE:COL'
    | str replace --all --regex 'rustc-ice-\d{4}-\d{2}-\d{2}T\d{2}_\d{2}_\d{2}-\d+' 'rustc-ice-TIMESTAMP'
    | str replace --all --regex '/tmp/co2-ct-dir-\w+' '/tmp/co2-ct-dir'
    | lines
    | where {|line| $line !~ 'stack backtrace' }
    | where {|line| $line !~ '^\s+\d+:' }
    | where {|line| $line !~ '^\s+at ' }
    | where {|line| $line !~ 'query stack' }
    | where {|line| ($line | str trim) != '' }
    | str join (char nl)
    | str trim
}

let test_dir = $env.CO2_TEST_DIR
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "break_co2")

# ---- co2cc ----
let c_src = ($test_dir | path join "test.c")
"break co2;
int x;
" | save -f $c_src

let co2cc_result = (do { ^co2cc $c_src -o ($test_dir | path join "out") } | complete)
let co2cc_stderr = (normalize $co2cc_result.stderr)
let co2cc_status = $co2cc_result.exit_code

# ---- co2rustc / co2rustdoc / co2miri (share host.rs/host.co2) ----
let host_rs = ($test_dir | path join "host.rs")
"#![language(co2)]
" | save -f $host_rs

let co2_src = ($test_dir | path join "host.co2")
"break co2;
fn main() {}
" | save -f $co2_src

let co2rustc_result = (do { ^co2rustc $host_rs -o ($test_dir | path join "out2") } | complete)
let co2rustc_stderr = (normalize $co2rustc_result.stderr)
let co2rustc_status = $co2rustc_result.exit_code

let co2rustdoc_result = (do { ^co2rustdoc $host_rs --crate-name test } | complete)
let co2rustdoc_stderr = (normalize $co2rustdoc_result.stderr)
let co2rustdoc_status = $co2rustdoc_result.exit_code

let co2miri_result = (do { ^co2miri $host_rs } | complete)
let co2miri_stderr = (normalize $co2miri_result.stderr)
let co2miri_status = $co2miri_result.exit_code

# ---- snapshot checks ----
if $co2cc_status != 101 {
    print $"FAIL: co2cc exit code expected 101, got ($co2cc_status)"
    exit 1
}
if $co2rustc_status != 101 {
    print $"FAIL: co2rustc exit code expected 101, got ($co2rustc_status)"
    exit 1
}
if $co2rustdoc_status != 101 {
    print $"FAIL: co2rustdoc exit code expected 101, got ($co2rustdoc_status)"
    exit 1
}
if $co2miri_status != 101 {
    print $"FAIL: co2miri exit code expected 101, got ($co2miri_status)"
    exit 1
}

assert-snapshot "co2cc stderr" $co2cc_stderr ($expected_dir | path join "co2cc.stderr.snapshot")
assert-snapshot "co2rustc stderr" $co2rustc_stderr ($expected_dir | path join "co2rustc.stderr.snapshot")
assert-snapshot "co2rustdoc stderr" $co2rustdoc_stderr ($expected_dir | path join "co2rustdoc.stderr.snapshot")
assert-snapshot "co2miri stderr" $co2miri_stderr ($expected_dir | path join "co2miri.stderr.snapshot")

print "break_co2 test passed"
exit 0
