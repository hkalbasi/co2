#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
cd ($test_dir | path join "demo")

let all = (do { ^co2cargo test } | complete)
if $all.exit_code != 101 {
    print $"expected failing all-tests run, got: ($all | to json -r)"
    exit 2
}
if ($all.stdout | str contains "running 4 tests") == false {
    print $"missing all-tests count in stdout.\nStdout: ($all.stdout)\nStderr: ($all.stderr)"
    exit 3
}
if ($all.stdout | str contains "test smoke_test ... ok") == false {
    print $"missing passing test output: ($all.stdout)"
    exit 4
}
if ($all.stdout | str contains "test failing_test ... FAILED") == false {
    print $"missing failing test output: ($all.stdout)"
    exit 5
}

let passing = (do { ^co2cargo test smoke_test } | complete)
if $passing.exit_code != 0 {
    print $"expected passing filtered run, got: ($passing | to json -r)"
    exit 6
}
if ($passing.stdout | str contains "running 2 tests") == false {
    print $"missing filtered passing count: ($passing.stdout)"
    exit 7
}
if ($passing.stdout | str contains "test smoke_test ... ok") == false {
    print $"missing filtered passing output: ($passing.stdout)"
    exit 8
}
if ($passing.stdout | str contains "test foo::smoke_test ... ok") == false {
    print $"missing nested passing output: ($passing.stdout)"
    exit 13
}
if ($passing.stdout | str contains "failing_test") == true {
    print $"unexpected failing test in passing filter output: ($passing.stdout)"
    exit 9
}

let failing = (do { ^co2cargo test failing_test } | complete)
if $failing.exit_code != 101 {
    print $"expected failing filtered run, got: ($failing | to json -r)"
    exit 10
}
if ($failing.stdout | str contains "running 1 test") == false {
    print $"missing filtered failing count: ($failing.stdout)"
    exit 11
}
if ($failing.stdout | str contains "test failing_test ... FAILED") == false {
    print $"missing filtered failing output: ($failing.stdout)"
    exit 12
}
