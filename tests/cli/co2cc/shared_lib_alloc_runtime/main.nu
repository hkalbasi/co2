#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let lib_src = ($test_dir | path join "lib_vec.c")
let main_src = ($test_dir | path join "main.c")
let lib_o = ($test_dir | path join "lib_vec.o")
let lib_path = ($test_dir | path join "libvla.so")
let app = ($test_dir | path join "vla_app")

let compile_obj = (do {
    ^co2cc -O2 -fPIC -c $lib_src -o $lib_o
} | complete)
if $compile_obj.exit_code != 0 {
    print $"object compile failed: ($compile_obj.stderr)"
    exit 1
}

let link_lib = (do {
    ^co2cc -shared $lib_o -Wl,--no-undefined -Wl,-soname,libvla.so -o $lib_path
} | complete)
if $link_lib.exit_code != 0 {
    print $"shared library link failed: ($link_lib.stderr)"
    exit 2
}

let compile_app = (do {
    ^co2cc $main_src -L $test_dir -lvla -o $app
} | complete)
if $compile_app.exit_code != 0 {
    print $"linked binary compile failed: ($compile_app.stderr)"
    exit 3
}

let run = (with-env { LD_LIBRARY_PATH: $test_dir } {
    do { ^$app } | complete
})
if $run.exit_code != 0 {
    print $"binary exited with ($run.exit_code): ($run.stderr)"
    exit 4
}
if ($run.stdout | str trim) != "15" {
    print $"binary stdout mismatch: ($run.stdout | str trim)"
    exit 5
}

exit 0
