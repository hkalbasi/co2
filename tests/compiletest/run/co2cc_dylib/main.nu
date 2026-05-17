#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let lib_path = ($test_dir | path join "libco2cc_dylib.so")
let linked_app = ($test_dir | path join "linked_app")
let dlopen_app = ($test_dir | path join "dlopen_app")

let compile_lib = (do {
    ^co2cc -shared -fPIC ($test_dir | path join "libmath.c") -o $lib_path
} | complete)
if $compile_lib.exit_code != 0 {
    print $"shared library compile failed: ($compile_lib.stderr)"
    exit 1
}

let compile_linked = (do {
    ^co2cc ($test_dir | path join "linked_main.c") -L $test_dir -lco2cc_dylib -o $linked_app
} | complete)
if $compile_linked.exit_code != 0 {
    print $"linked binary compile failed: ($compile_linked.stderr)"
    exit 2
}

let linked_run = (with-env { LD_LIBRARY_PATH: $test_dir } {
    do { ^$linked_app } | complete
})
if $linked_run.exit_code != 0 {
    print $"linked binary failed: ($linked_run.stderr)"
    exit 3
}
if ($linked_run.stdout | str trim) != "49" {
    print $"linked binary stdout mismatch: ($linked_run.stdout | str trim)"
    exit 4
}

let compile_dlopen = (do {
    ^co2cc ($test_dir | path join "dlopen_main.c") -ldl -o $dlopen_app
} | complete)
if $compile_dlopen.exit_code != 0 {
    print $"dlopen binary compile failed: ($compile_dlopen.stderr)"
    exit 5
}

let dlopen_run = (with-env { LD_LIBRARY_PATH: $test_dir } {
    do { ^$dlopen_app } | complete
})
if $dlopen_run.exit_code != 0 {
    print $"dlopen binary failed: ($dlopen_run.stderr)"
    exit 6
}
if ($dlopen_run.stdout | str trim) != "50" {
    print $"dlopen binary stdout mismatch: ($dlopen_run.stdout | str trim)"
    exit 7
}

exit 0
