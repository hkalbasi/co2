#@ run-status: 0

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")

let no_args = (do { ^$co2cc } | complete)
if $no_args.exit_code != 2 or ($no_args.stderr | str contains "missing input file") == false {
    print $"co2cc no-args path failed: ($no_args | to json -r)"
    exit 1
}

let missing_o = (do { ^$co2cc -o } | complete)
if $missing_o.exit_code != 2 or ($missing_o.stderr | str contains "missing argument after -o") == false {
    print $"co2cc missing -o argument path failed: ($missing_o | to json -r)"
    exit 1
}

"int first(void) { return 0; }\n" | save -f first.c
"int second(void) { return 0; }\n" | save -f second.c

let invalid_object_inputs = (do { ^$co2cc -c first.c second.c -o both.o } | complete)
if $invalid_object_inputs.exit_code != 2 or ($invalid_object_inputs.stderr | str contains "object emission mode expects exactly one C input file") == false {
    print $"co2cc invalid -c input validation failed: ($invalid_object_inputs | to json -r)"
    exit 1
}

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
