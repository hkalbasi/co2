#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "trivial.c")
let co2cc_asm = ($test_dir | path join "co2cc_trivial.s")
let gcc_asm = ($test_dir | path join "gcc_trivial.s")
let co2cc_bin = ($test_dir | path join "co2cc_trivial")
let gcc_bin = ($test_dir | path join "gcc_trivial")

# compile to assembly with -S -O2
let co2cc_compile = (do { co2cc -S -O2 $source -o $co2cc_asm } | complete)
if $co2cc_compile.exit_code != 0 {
    print $"co2cc -S -O2 failed: ($co2cc_compile.stderr)"
    exit 1
}

let gcc_compile = (do { gcc -S -O2 $source -o $gcc_asm } | complete)
if $gcc_compile.exit_code != 0 {
    print $"gcc -S -O2 failed: ($gcc_compile.stderr)"
    exit 1
}

# verify assembly output is non-empty
let co2cc_asm_size = ((ls $co2cc_asm).0.size | into int)
if $co2cc_asm_size <= 10 {
    print $"co2cc -S produced suspiciously small assembly: ($co2cc_asm_size) bytes"
    exit 1
}

let gcc_asm_size = ((ls $gcc_asm).0.size | into int)
if $gcc_asm_size <= 10 {
    print $"gcc -S produced suspiciously small assembly: ($gcc_asm_size) bytes"
    exit 1
}

# assemble and link both .s files, then verify they produce same exit code
let co2cc_link = (do { gcc $co2cc_asm -o $co2cc_bin } | complete)
if $co2cc_link.exit_code != 0 {
    print $"assembling/linking co2cc asm failed: ($co2cc_link.stderr)"
    exit 1
}

let gcc_link = (do { gcc $gcc_asm -o $gcc_bin } | complete)
if $gcc_link.exit_code != 0 {
    print $"assembling/linking gcc asm failed: ($gcc_link.stderr)"
    exit 1
}

let co2cc_run = (do { ^$co2cc_bin } | complete)
let gcc_run = (do { ^$gcc_bin } | complete)

if $co2cc_run.exit_code != $gcc_run.exit_code {
    print $"co2cc asm produced exit ($co2cc_run.exit_code), gcc asm produced exit ($gcc_run.exit_code)"
    exit 1
}

if $co2cc_run.exit_code != 42 {
    print $"expected exit 42, got ($co2cc_run.exit_code)"
    exit 1
}

exit 0
