#@ run-status: 0

# x86 SIMD arch flags must control codegen and predefined macros: without any
# -m/-march flag the AVX2+FMA source must fail to compile (target features off)
# and __SSE4_2__/__AVX__ etc. must stay undefined, while -mavx2 -mfma and
# -march=native must each compile it into a working binary with the macros defined.

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "simd.c")
let macros = ($test_dir | path join "simd_macros.c")
let app_noflag = ($test_dir | path join "app_noflag")
let app_mflags = ($test_dir | path join "app_mflags")
let app_native = ($test_dir | path join "app_native")
let macros_noflag = ($test_dir | path join "macros_noflag")
let macros_mflags = ($test_dir | path join "macros_mflags")
let macros_native = ($test_dir | path join "macros_native")

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

# Case 4: predefined macros without arch flags -> __SSE4_2__/__AVX__ etc.
# must be absent, so the macro check must fail to compile.
let macros_noflag_r = (do { co2cc -O2 $macros -o $macros_noflag } | complete)
if $macros_noflag_r.exit_code == 0 {
    print "co2cc without arch flags unexpectedly defined AVX macros"
    exit 6
}

# Case 5: explicit -mavx2 -mfma -> macro check must compile and run.
let macros_mflags_r = (do { co2cc -O2 -mavx2 -mfma $macros -o $macros_mflags } | complete)
if $macros_mflags_r.exit_code != 0 {
    print $"co2cc -mavx2 -mfma macro check failed: ($macros_mflags_r.stderr)"
    exit 7
}

let macros_mflags_run = (do { ^$macros_mflags } | complete)
if $macros_mflags_run.exit_code != 0 {
    print $"macro check binary built with -mavx2 -mfma exited with ($macros_mflags_run.exit_code): ($macros_mflags_run.stdout) ($macros_mflags_run.stderr)"
    exit 8
}

# Case 6: -march=native -> macro check must compile and run.
let macros_native_r = (do { co2cc -O2 -march=native $macros -o $macros_native } | complete)
if $macros_native_r.exit_code != 0 {
    print $"co2cc -march=native macro check failed: ($macros_native_r.stderr)"
    exit 9
}

let macros_native_run = (do { ^$macros_native } | complete)
if $macros_native_run.exit_code != 0 {
    print $"macro check binary built with -march=native exited with ($macros_native_run.exit_code): ($macros_native_run.stdout) ($macros_native_run.stderr)"
    exit 10
}

exit 0
