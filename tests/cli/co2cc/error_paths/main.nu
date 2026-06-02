#@ run-status: 0

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")
let test_dir = $env.CO2_TEST_DIR
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cc" "error_paths")

def assert-snapshot [name: string, actual: string, snapshot_name: string] {
    let snapshot = (open ($expected_dir | path join $snapshot_name) | str trim)
    if $actual != $snapshot {
        print $"FAIL: ($name) mismatch!"
        print "--- GOT ---"
        print $actual
        print "--- EXPECTED ---"
        print $snapshot
        exit 1
    }
}

# ---- no args ----
let no_args = (do { ^$co2cc } | complete)
if $no_args.exit_code != 2 {
    print $"FAIL: co2cc no-args exit code expected 2, got ($no_args.exit_code)"
    exit 1
}
assert-snapshot "no_args" ($no_args.stderr | str trim) "no_args.stderr.snapshot"

# ---- -h and --help ----
let help_h = (do { ^$co2cc -h } | complete)
if $help_h.exit_code != 0 {
    print $"FAIL: co2cc -h exit code expected 0, got ($help_h.exit_code)"
    exit 1
}
if ($help_h.stdout | str trim) != "" {
    print "FAIL: co2cc -h expected empty stdout"
    exit 1
}
assert-snapshot "help_h" ($help_h.stderr | str trim) "help.stderr.snapshot"

let help_help = (do { ^$co2cc --help } | complete)
if $help_help.exit_code != 0 {
    print $"FAIL: co2cc --help exit code expected 0, got ($help_help.exit_code)"
    exit 1
}
if ($help_help.stdout | str trim) != "" {
    print "FAIL: co2cc --help expected empty stdout"
    exit 1
}
assert-snapshot "help_help" ($help_help.stderr | str trim) "help.stderr.snapshot"

# ---- missing -o argument ----
let missing_o = (do { ^$co2cc -o } | complete)
if $missing_o.exit_code != 2 {
    print $"FAIL: co2cc missing -o exit code expected 2, got ($missing_o.exit_code)"
    exit 1
}
assert-snapshot "missing_o" ($missing_o.stderr | str trim) "missing_o.stderr.snapshot"

# ---- invalid -c with multiple inputs ----
"int first(void) { return 0; }\n" | save -f first.c
"int second(void) { return 0; }\n" | save -f second.c

let invalid_object_inputs = (do { ^$co2cc -c first.c second.c -o both.o } | complete)
if $invalid_object_inputs.exit_code != 2 {
    print $"FAIL: co2cc invalid -c exit code expected 2, got ($invalid_object_inputs.exit_code)"
    exit 1
}
assert-snapshot "invalid_object_inputs" ($invalid_object_inputs.stderr | str trim) "invalid_object_inputs.stderr.snapshot"

# ---- .co2 file (should suggest co2cargo/co2rustc) ----
let foo_dot_co2 = (do { ^$co2cc ./foo.co2 } | complete)
if $foo_dot_co2.exit_code != 2 {
    print $"FAIL: co2cc ./foo.co2 exit code expected 2, got ($foo_dot_co2.exit_code)"
    exit 1
}
assert-snapshot "foo_dot_co2" ($foo_dot_co2.stderr | str trim) "foo_dot_co2.stderr.snapshot"

# ---- non-existing input file ----
let non_existing = (do { ^$co2cc ./non-existing.c } | complete)
if $non_existing.exit_code != 2 {
    print $"FAIL: co2cc ./non-existing.c exit code expected 2, got ($non_existing.exit_code)"
    exit 1
}
assert-snapshot "non_existing_c" ($non_existing.stderr | str trim) "non_existing_c.stderr.snapshot"

# ---- combined preprocessor flags ----
mkdir inc
mkdir quote
mkdir sys

"#define FROM_I 10\n" | save -f (["inc" "from_i.h"] | path join)
"#define FROM_QUOTE 20\n" | save -f (["quote" "quoted.h"] | path join)
"#define FROM_SYSTEM 30\n" | save -f (["sys" "system.h"] | path join)
"#define FORCED 40\n" | save -f forced.h

"#include \"from_i.h\"
#include \"quoted.h\"
#include <system.h>

#ifndef FORCED
#error \"forced include missing\"
#endif

#ifndef VALUE
#error \"VALUE missing\"
#endif

int answer(void) {
    return FROM_I + FROM_QUOTE + FROM_SYSTEM + FORCED + VALUE;
}
" | save -f flags.c

let flags_compile = (
    do {
        with-env { RUSTFLAGS: "--cap-lints allow" } {
            ^$co2cc -c -nostdinc -undef -Iinc -iquotequote -isystemsys -include forced.h -UVALUE -DVALUE=7 -std=c11 flags.c -o flags.o
        }
    } | complete
)
if $flags_compile.exit_code != 0 {
    print $"co2cc combined preprocessor flags failed: ($flags_compile | to json -r)"
    exit 1
}

if ("flags.o" | path exists) == false {
    print "co2cc did not produce the object file for combined flag parsing"
    exit 1
}

let symbol_table = (do { nm flags.o } | complete)
if $symbol_table.exit_code != 0 or ($symbol_table.stdout | str contains " answer") == false {
    print $"co2cc combined flag object output is missing answer(): ($symbol_table | to json -r)"
    exit 1
}
