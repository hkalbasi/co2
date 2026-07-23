#@ run-status: 0

cd $env.CO2_TEST_DIR
let result = (do { ^co2cargo build } | complete)
if $result.exit_code != 0 {
    print $"FAIL: build failed unexpectedly"
    print ($result.stderr | str trim)
    exit 1
}
let result = (do { ^co2cargo run } | complete)
if $result.exit_code != 0 {
    print $"FAIL: run failed unexpectedly"
    print ($result.stderr | str trim)
    exit 1
}
let result = (do { ^co2cargo miri run } | complete)
if $result.exit_code != 0 {
    print $"FAIL: miri run failed unexpectedly"
    print ($result.stderr | str trim)
    exit 1
}
print "PASS"
