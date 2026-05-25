#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "hello_world.c")
let co2cc_bin = ($test_dir | path join "hello_co2cc")
let gcc_bin = ($test_dir | path join "hello_gcc")

let co2cc_compile = (do { co2cc -O2 $source -o $co2cc_bin } | complete)
if $co2cc_compile.exit_code != 0 {
    print $"co2cc -O2 failed: ($co2cc_compile.stderr)"
    exit 1
}

let gcc_compile = (do { gcc -O2 $source -o $gcc_bin } | complete)
if $gcc_compile.exit_code != 0 {
    print $"gcc -O2 failed: ($gcc_compile.stderr)"
    exit 1
}

let co2cc_run = (do { ^$co2cc_bin } | complete)
if $co2cc_run.exit_code != 0 {
    print $"co2cc binary exited with ($co2cc_run.exit_code): ($co2cc_run.stderr)"
    exit 1
}

let gcc_run = (do { ^$gcc_bin } | complete)
if $gcc_run.exit_code != 0 {
    print $"gcc binary exited with ($gcc_run.exit_code): ($gcc_run.stderr)"
    exit 1
}

let co2cc_size = ((ls $co2cc_bin).0.size | into int)
let gcc_size = ((ls $gcc_bin).0.size | into int)

print $"co2cc size: ($co2cc_size)"
print $"gcc size: ($gcc_size)"

if $co2cc_size >= $gcc_size {
    print $"expected co2cc -O2 binary to be smaller than gcc -O2 binary"
    exit 1
}

exit 0
