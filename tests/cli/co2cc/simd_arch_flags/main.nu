#@ run-status: 0

# x86 SIMD arch flags must control codegen: without any -m/-march flag the
# AVX2+FMA source must fail to compile (target features off), while
# -mavx2 -mfma and -march=native must each compile it into a working binary.

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "simd.c")
let app_noflag = ($test_dir | path join "app_noflag")
let app_mflags = ($test_dir | path join "app_mflags")
let app_native = ($test_dir | path join "app_native")

# Case 1: no arch flags -> compile must fail.
let noflag = (do { co2cc -O2 $source -o $app_noflag } | complete)
if $noflag.exit_code == 0 {
    print "co2cc without arch flags unexpectedly compiled AVX2 code"
    exit 1
}

# Case 2: explicit -mavx2 -mfma -> compile and run must succeed.
let mflags = (do { co2cc -O2 -mavx2 -mfma $source -o $app_mflags } | complete)
if $mflags.exit_code != 0 {
    print $"co2cc -mavx2 -mfma failed: ($mflags.stderr)"
    exit 2
}

let mflags_run = (do { ^$app_mflags } | complete)
if $mflags_run.exit_code != 0 {
    print $"binary built with -mavx2 -mfma exited with ($mflags_run.exit_code): ($mflags_run.stdout) ($mflags_run.stderr)"
    exit 3
}

# Case 3: -march=native -> compile and run must succeed.
let native = (do { co2cc -O2 -march=native $source -o $app_native } | complete)
if $native.exit_code != 0 {
    print $"co2cc -march=native failed: ($native.stderr)"
    exit 4
}

let native_run = (do { ^$app_native } | complete)
if $native_run.exit_code != 0 {
    print $"binary built with -march=native exited with ($native_run.exit_code): ($native_run.stdout) ($native_run.stderr)"
    exit 5
}

exit 0
