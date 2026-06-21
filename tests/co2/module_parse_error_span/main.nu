#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
cd $test_dir

let status = (do { ^co2cargo -q build } | complete)

# Compare Cargo stderr against the expected rendered diagnostic.
let actual = ($status.stderr | str trim)
assert-snapshot "stderr" $actual ($test_dir | path join "stderr.snapshot")
exit 0
