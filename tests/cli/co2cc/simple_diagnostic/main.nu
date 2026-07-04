#@ run-status: 0

use ./snapshot-utils.nu *

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")
let test_dir = $env.CO2_TEST_DIR
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cc" "simple_diagnostic")

# Test 1: run from the same directory
cd $test_dir

let result1 = (do { ^$co2cc broken.c -o broken } | complete)
if $result1.exit_code != 5 {
    print $"FAIL: co2cc exit code expected 5, got ($result1.exit_code)"
    exit 1
}

let actual1 = ($result1.stderr | str replace -a -r '\x1b\[[0-9;]*[a-zA-Z]' '' | str trim)
assert-snapshot "stderr" $actual1 ($expected_dir | path join "stderr.snapshot")

# Test 2: run from a subdirectory with ../broken.c
let subdir = ($test_dir | path join "subdir_test")
mkdir $subdir
cd $subdir

let result2 = (do { ^$co2cc ../broken.c -o ../broken } | complete)
if $result2.exit_code != 5 {
    print $"FAIL: co2cc exit code expected 5, got ($result2.exit_code)"
    exit 1
}

let actual2 = ($result2.stderr | str replace -a -r '\x1b\[[0-9;]*[a-zA-Z]' '' | str trim)
assert-snapshot "stderr_subdir" $actual2 ($expected_dir | path join "stderr_subdir.snapshot")

exit 0
