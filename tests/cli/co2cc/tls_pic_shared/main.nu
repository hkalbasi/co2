#@ run-status: 0

# `static __thread` must use a PIC-compatible TLS model when building a
# shared object. Today co2cc always emits local-exec (R_X86_64_TPOFF32),
# which ld refuses under -shared. Minimal jq blocker repro:
# `co2cc -fPIC -shared t_tls.c`.

let test_dir = $env.CO2_TEST_DIR
let lib_src = ($test_dir | path join "t_tls.c")
let main_src = ($test_dir | path join "main.c")
let lib_path = ($test_dir | path join "libtls.so")
let app = ($test_dir | path join "tls_app")

let compile_lib = (do {
    ^co2cc -fPIC -shared $lib_src -o $lib_path
} | complete)
if $compile_lib.exit_code != 0 {
    print $"shared library compile failed: ($compile_lib.stderr)"
    exit 1
}

let compile_app = (do {
    ^co2cc $main_src -L $test_dir -ltls -o $app
} | complete)
if $compile_app.exit_code != 0 {
    print $"linked binary compile failed: ($compile_app.stderr)"
    exit 2
}

let run = (with-env { LD_LIBRARY_PATH: $test_dir } {
    do { ^$app } | complete
})
if $run.exit_code != 0 {
    print $"tls binary exited with ($run.exit_code): ($run.stderr)"
    exit 3
}
if ($run.stdout | str trim) != "tls-ok" {
    print $"tls binary stdout mismatch: ($run.stdout | str trim)"
    exit 4
}

exit 0
