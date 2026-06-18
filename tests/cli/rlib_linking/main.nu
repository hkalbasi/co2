#@ run-status: 0

let co2rustc = ($env.CO2_BIN_DIR | path join "co2rustc")
let test_dir = $env.CO2_TEST_DIR

let foo_rlib = ($test_dir | path join "libfoo.rlib")
let bar_bin = ($test_dir | path join "bar_bin")

let compile_foo = (do { ^$co2rustc foo.rs --crate-type=lib --crate-name=foo -o $foo_rlib } | complete)
if $compile_foo.exit_code != 0 {
    print $"compiling foo lib failed: ($compile_foo.stderr)"
    exit 1
}
if ($foo_rlib | path exists) == false {
    print "co2rustc did not produce libfoo.rlib"
    exit 1
}

let compile_bar = (do { ^$co2rustc bar.rs --extern $"foo=($foo_rlib)" -o $bar_bin } | complete)
if $compile_bar.exit_code != 0 {
    print $"compiling bar bin failed: ($compile_bar.stderr)"
    exit 1
}
if ($bar_bin | path exists) == false {
    print "co2rustc did not produce the binary"
    exit 1
}

let run_bar = (do { ^$bar_bin } | complete)
if $run_bar.exit_code != 52 {
    print $"bar binary exit code expected 52, got ($run_bar.exit_code), stderr: ($run_bar.stderr)"
    exit 1
}

print "PASS"
exit 0
