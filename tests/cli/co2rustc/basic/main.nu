#@ run-status: 0

let co2rustc = ($env.CO2_BIN_DIR | path join "co2rustc")
let test_dir = $env.CO2_TEST_DIR

# ---- normal Rust file ----
let compile_rust = (do { ^$co2rustc hello.rs -o hello_rust } | complete)
if $compile_rust.exit_code != 0 {
    print $"compiling normal Rust failed: ($compile_rust.stderr)"
    exit 1
}
if ("hello_rust" | path exists) == false {
    print "co2rustc did not produce the binary for normal Rust"
    exit 1
}

let run_rust = (do { ./hello_rust } | complete)
if $run_rust.exit_code != 0 {
    print $"running normal Rust binary failed: ($run_rust.stderr)"
    exit 1
}
if ($run_rust.stdout | str trim) != "hello from rust" {
    print $"normal Rust binary output mismatch: ($run_rust | to json -r)"
    exit 1
}

# ---- co2 host file ----
let compile_co2 = (do { ^$co2rustc basic.rs -o basic_co2 } | complete)
if $compile_co2.exit_code != 0 {
    print $"compiling co2 host failed: ($compile_co2.stderr)"
    exit 1
}
if ("basic_co2" | path exists) == false {
    print "co2rustc did not produce the binary for co2 host"
    exit 1
}

let run_co2 = (do { ./basic_co2 } | complete)
if $run_co2.exit_code != 0 {
    print $"running co2 binary failed: ($run_co2 | to json -r)"
    exit 1
}
if ($run_co2.stdout | str trim) != "Hello from co2" {
    print $"co2 binary output mismatch: ($run_co2 | to json -r)"
    exit 1
}

# ---- direct .co2 file (no sibling .rs) ----
let compile_direct = (do { ^$co2rustc direct.co2 -o direct_co2 } | complete)
if $compile_direct.exit_code != 0 {
    print $"compiling direct .co2 failed: ($compile_direct.stderr)"
    exit 1
}
if ("direct_co2" | path exists) == false {
    print "co2rustc did not produce the binary for direct .co2"
    exit 1
}

let run_direct = (do { ./direct_co2 } | complete)
if $run_direct.exit_code != 0 {
    print $"running direct .co2 binary failed: ($run_direct | to json -r)"
    exit 1
}
if ($run_direct.stdout | str trim) != "direct co2" {
    print $"direct .co2 binary output mismatch: ($run_direct | to json -r)"
    exit 1
}
