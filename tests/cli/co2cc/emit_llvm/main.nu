#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "trivial.c")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cc" "emit_llvm")

# ---- -S -emit-llvm (LLVM IR text, clang-compatible) ----
let ll_file = ($test_dir | path join "co2cc_trivial.ll")
let compile = (do { co2cc -S -O2 -emit-llvm $source -o $ll_file } | complete)
if $compile.exit_code != 0 {
    print $"co2cc -S -O2 -emit-llvm failed: ($compile.stderr)"
    exit 1
}

# verify LLVM IR output is non-empty
let ll_size = ((ls $ll_file).0.size | into int)
if $ll_size <= 10 {
    print $"co2cc -S -O2 -emit-llvm produced suspiciously small LLVM IR: ($ll_size) bytes"
    exit 1
}

# ---- -c -emit-llvm (bitcode, clang-compatible) ----
let bc_out = ($test_dir | path join "co2cc_trivial_c.bc")
let compile_c = (do { co2cc -c -O2 -emit-llvm $source -o $bc_out } | complete)
if $compile_c.exit_code != 0 {
    print $"co2cc -c -O2 -emit-llvm failed: ($compile_c.stderr)"
    exit 1
}

# verify bitcode output is non-empty and starts with the LLVM bitcode magic
let bc_bytes = (open --raw $bc_out | into binary)
let bc_size = ($bc_bytes | length)
if $bc_size <= 10 {
    print $"co2cc -c -O2 -emit-llvm produced suspiciously small bitcode: ($bc_size) bytes"
    exit 1
}
let bc_magic = ($bc_bytes | first 4)
if $bc_magic != 0x[4243c0de] {
    print $"co2cc -c -O2 -emit-llvm output is not valid LLVM bitcode; magic=($bc_magic)"
    exit 1
}

# ---- -emit-llvm without -S/-c must error like clang ----
let linking_err = (
    do { ^co2cc -O2 -emit-llvm $source -o ($test_dir | path join "co2cc_trivial_link") } | complete
)
if $linking_err.exit_code != 1 {
    print $"co2cc -emit-llvm without -S/-c exit code expected 1, got ($linking_err.exit_code)"
    exit 1
}
assert-snapshot "emit-llvm linking error" $linking_err.stderr ($expected_dir | path join "linking_error.stderr.snapshot")

# snapshot check: compare normalized LLVM IR (skip source_filename line with temp path)
let ll_text = (open $ll_file)
let normalize = {|s|
    $s | lines | where {|line|
        let t = ($line | str trim)
        ($t | str starts-with "source_filename") == false
    } | str join "\n"
}
let actual = (do $normalize $ll_text)
assert-snapshot "emit-llvm IR" $actual ($expected_dir | path join "trivial.snapshot")

exit 0
