#@ run-status: 0

# C99 plain `inline` in a header must not emit an external definition in
# every TU. Today co2cc codegens it like a normal function, so linking any
# two TUs fails with "duplicate symbol". `static inline` is fine. gnulib hits
# this via _GL_INLINE -> plain inline (streq, rpl_realloc, ...).
#
# Behavior-only test (no `nm`): wrong codegen must show up as a link error
# or a wrong exit code.
#
# Case 1 (-O2): everything inlines, so linking two plain TUs must succeed.
# Case 2 (-O0): nothing inlines; the single `extern inline` TU in tu_ext.c
# provides the external definition the other two TUs call into.
# Case 3 (-O0): tu_decl.c never sees the inline definition, only a plain
# declaration, so it links only if the `extern inline` TU really exports
# the symbol. Per-TU local (`static`-like) copies fail here with
# "undefined symbol".

let test_dir = $env.CO2_TEST_DIR

let tu1 = ($test_dir | path join "tu1.c")
let tu2 = ($test_dir | path join "tu2.c")
let tu_ext = ($test_dir | path join "tu_ext.c")
let tu_decl = ($test_dir | path join "tu_decl.c")
let app_co2 = ($test_dir | path join "app_co2")
let app_gcc = ($test_dir | path join "app_gcc")
let app_ext_co2 = ($test_dir | path join "app_ext_co2")
let app_ext_gcc = ($test_dir | path join "app_ext_gcc")
let app_decl_co2 = ($test_dir | path join "app_decl_co2")
let app_decl_gcc = ($test_dir | path join "app_decl_gcc")

# Case 1: plain inline, -O2.
let gcc_link = (do { ^gcc -O2 $tu1 $tu2 -o $app_gcc } | complete)
if $gcc_link.exit_code != 0 {
    print $"gcc link failed [oracle]: ($gcc_link.stderr)"
    exit 1
}

let co2_link = (do { ^co2cc -O2 $tu1 $tu2 -o $app_co2 } | complete)
if $co2_link.exit_code != 0 {
    print $"co2cc link failed [duplicate myinlinefn?]: ($co2_link.stderr)"
    exit 2
}

let gcc_run = (do { ^$app_gcc } | complete)
if $gcc_run.exit_code != 0 {
    print $"gcc-built app exited with ($gcc_run.exit_code)"
    exit 1
}

let co2_run = (do { ^$app_co2 } | complete)
if $co2_run.exit_code != 0 {
    print $"co2cc-built app exited with ($co2_run.exit_code), expected 0"
    exit 3
}

# Case 2: extern inline instantiation, -O0.
let gcc_ext_link = (do { ^gcc -O0 $tu1 $tu2 $tu_ext -o $app_ext_gcc } | complete)
if $gcc_ext_link.exit_code != 0 {
    print $"gcc -O0 link failed [oracle]: ($gcc_ext_link.stderr)"
    exit 1
}

let co2_ext_link = (do { ^co2cc -O0 $tu1 $tu2 $tu_ext -o $app_ext_co2 } | complete)
if $co2_ext_link.exit_code != 0 {
    print $"co2cc -O0 link failed [duplicate myinlinefn?]: ($co2_ext_link.stderr)"
    exit 4
}

let gcc_ext_run = (do { ^$app_ext_gcc } | complete)
if $gcc_ext_run.exit_code != 0 {
    print $"gcc-built extern app exited with ($gcc_ext_run.exit_code)"
    exit 1
}

let co2_ext_run = (do { ^$app_ext_co2 } | complete)
if $co2_ext_run.exit_code != 0 {
    print $"co2cc-built extern app exited with ($co2_ext_run.exit_code), expected 0"
    exit 5
}

# Case 3: declaration-only caller needs the exported symbol, -O0.
let gcc_decl_link = (do { ^gcc -O0 $tu_decl $tu_ext -o $app_decl_gcc } | complete)
if $gcc_decl_link.exit_code != 0 {
    print $"gcc -O0 link failed [oracle]: ($gcc_decl_link.stderr)"
    exit 1
}

let co2_decl_link = (do { ^co2cc -O0 $tu_decl $tu_ext -o $app_decl_co2 } | complete)
if $co2_decl_link.exit_code != 0 {
    print $"co2cc -O0 link failed [undefined myinlinefn?]: ($co2_decl_link.stderr)"
    exit 6
}

let gcc_decl_run = (do { ^$app_decl_gcc } | complete)
if $gcc_decl_run.exit_code != 0 {
    print $"gcc-built decl app exited with ($gcc_decl_run.exit_code)"
    exit 1
}

let co2_decl_run = (do { ^$app_decl_co2 } | complete)
if $co2_decl_run.exit_code != 0 {
    print $"co2cc-built decl app exited with ($co2_decl_run.exit_code), expected 0"
    exit 7
}

exit 0
