#@ run-status: 0

use snapshot-utils.nu ["assert-snapshot"]

let source = ($env.CO2_TEST_DIR | path join "trivial.c")
let snap_path = ($env.CO2_TEST_SOURCE_DIR | path join "trivial-expanded.txt")

# test stdout output
let co2cc_e = (do { co2cc -E $source } | complete)
if $co2cc_e.exit_code != 0 {
    print $"co2cc -E failed: ($co2cc_e.stderr)"
    exit 1
}
assert-snapshot "stdout output" $co2cc_e.stdout $snap_path

# test -o output
let co2cc_o_path = ($env.CO2_TEST_DIR | path join "trivial.i")
let co2cc_o = (do { co2cc -E $source -o $co2cc_o_path } | complete)
if $co2cc_o.exit_code != 0 {
    print $"co2cc -E -o failed: ($co2cc_o.stderr)"
    exit 1
}
let co2cc_o_content = (open $co2cc_o_path)
assert-snapshot "-o output" $co2cc_o_content $snap_path

exit 0
