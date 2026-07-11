#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "trivial.c")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cc" "emit_llvm")

let ll_file = ($test_dir | path join "co2cc_trivial.ll")
let bc_file = ($test_dir | path join "co2cc_trivial.bc")
let bin_file = ($test_dir | path join "co2cc_trivial")

# compile to LLVM IR (clang-compatible: -S -emit-llvm emits .ll)
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

# assemble the LLVM IR with llvm-as to confirm it is valid (clang does the same)
let assemble = (do { ^llvm-as $ll_file -o $bc_file } | complete)
if $assemble.exit_code != 0 {
    print $"llvm-as of co2cc -S -emit-llvm output failed: ($assemble.stderr)"
    exit 1
}

# link and run, then verify the expected exit code (mirrors the -S asm test)
let link = (do { ^clang $bc_file -o $bin_file } | complete)
if $link.exit_code != 0 {
    print $"linking co2cc -S -emit-llvm output failed: ($link.stderr)"
    exit 1
}

let run = (do { ^$bin_file } | complete)
if $run.exit_code != 42 {
    print $"expected exit 42 for -S -emit-llvm, got ($run.exit_code)"
    exit 1
}

# ---- -c -emit-llvm (bitcode, clang-compatible) ----
let bc_out = ($test_dir | path join "co2cc_trivial_c.bc")
let compile_c = (do { co2cc -c -O2 -emit-llvm $source -o $bc_out } | complete)
if $compile_c.exit_code != 0 {
    print $"co2cc -c -O2 -emit-llvm failed: ($compile_c.stderr)"
    exit 1
}

# verify bitcode output is non-empty
let bc_size = ((ls $bc_out).0.size | into int)
if $bc_size <= 10 {
    print $"co2cc -c -O2 -emit-llvm produced suspiciously small bitcode: ($bc_size) bytes"
    exit 1
}

# bitcode must be valid: link it directly with clang (which accepts .bc), then run
let link_c = (do { ^clang $bc_out -o $bin_file } | complete)
if $link_c.exit_code != 0 {
    print $"linking co2cc -c -emit-llvm output failed: ($link_c.stderr)"
    exit 1
}

let run_c = (do { ^$bin_file } | complete)
if $run_c.exit_code != 42 {
    print $"expected exit 42 for -c -emit-llvm, got ($run_c.exit_code)"
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
