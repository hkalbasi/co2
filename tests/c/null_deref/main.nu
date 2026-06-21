#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let main_c = ($test_dir | path join "main.c")
"int main() {
    int *p = 0;
    return *p;
}
" | save -f $main_c

let bin = ($test_dir | path join "main_bin")
let compile = (do { co2cc $main_c -o $bin } | complete)
if $compile.exit_code != 0 {
    print $"compilation failed: ($compile.stderr)"
    exit 1
}

let run = (do { ^$bin } | complete)

# Normalize stderr
# 1. Thread ID/PID: thread '<unnamed>' (2768895) -> thread '<unnamed>' (PID)
# 2. Path: panicked at /abspath/main.c:3:12: -> panicked at main.c:3:12:
# 3. Strip backtrace note
# 4. Handle "Aborted (core dumped)" message from shell if it's there (usually not when using 'complete' in nu)

let stderr_normalized = (
    $run.stderr 
    | str replace --regex '\(\d+\) panicked' '(PID) panicked'
    | str replace --regex 'panicked at .+[/\\]main\.c:' 'panicked at main.c:'
    | str replace --regex 'note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\s*' ''
)

let stdout_normalized = $run.stdout
let status = $run.exit_code

let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "c" "null_deref")
let expected_status = (open ($expected_dir | path join "status.expected") | str trim | into int)

if $status != $expected_status {
    print $"FAIL: status mismatch. Expected ($expected_status), got ($status)"
    exit 1
}
assert-snapshot "stdout" $stdout_normalized ($expected_dir | path join "stdout.expected")
assert-snapshot "stderr" $stderr_normalized ($expected_dir | path join "stderr.expected")

print "c_null_deref test passed"
exit 0
