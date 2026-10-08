#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
cd $test_dir

let check = (do { ^co2cargo -q check } | complete)

if $check.exit_code != 0 {
    print "co2cargo check failed"
    print $check.stderr
    exit 1
}

exit 0
