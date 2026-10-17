#@ run-status: 0

# GNU inline attributes vs gcc.
#
# noinline: `__attribute__((noinline))` must survive -O2 (supported), also
#   when written after the `*` in a pointer return type (get_ptr).
# always_inline: `__attribute__((always_inline))` must force inlining at -O0
#   (not yet supported — co2cc strips it, so this case fails).
# Both use an opaque value (argc) so the call cannot fold away.

let test_dir = $env.CO2_TEST_DIR
let src = ($test_dir | path join "noinline.c")
let co2_asm = ($test_dir | path join "co2.s")
let gcc_asm = ($test_dir | path join "gcc.s")
let co2_bin = ($test_dir | path join "app_co2")
let gcc_bin = ($test_dir | path join "app_gcc")

let co2_compile = (do { ^co2cc -O2 -S $src -o $co2_asm } | complete)
if $co2_compile.exit_code != 0 {
    print $"co2cc -S -O2 failed: ($co2_compile.stderr)"
    exit 1
}

let gcc_compile = (do { ^gcc -O2 -S $src -o $gcc_asm } | complete)
if $gcc_compile.exit_code != 0 {
    print $"gcc -S -O2 failed [oracle]: ($gcc_compile.stderr)"
    exit 1
}

let gcc_text = (open $gcc_asm)
if $gcc_text !~ 'call.*target' {
    print "gcc -O2 did not keep a call to target [oracle broken?]"
    exit 1
}

let co2_text = (open $co2_asm)
if $co2_text !~ 'call.*target' {
    print "co2cc -O2 inlined target despite __attribute__((noinline))"
    exit 1
}

# Same attribute, but written after the `*`: must survive -O2 too.
if $gcc_text !~ 'call.*get_ptr' {
    print "gcc -O2 did not keep a call to get_ptr [oracle broken?]"
    exit 1
}

if $co2_text !~ 'call.*get_ptr' {
    print "co2cc -O2 inlined get_ptr despite __attribute__((noinline)) after '*'"
    exit 1
}

let co2_link = (do { ^co2cc -O2 $src -o $co2_bin } | complete)
if $co2_link.exit_code != 0 {
    print $"co2cc -O2 link failed: ($co2_link.stderr)"
    exit 1
}

let gcc_link = (do { ^gcc -O2 $src -o $gcc_bin } | complete)
if $gcc_link.exit_code != 0 {
    print $"gcc -O2 link failed [oracle]: ($gcc_link.stderr)"
    exit 1
}

let co2_run = (do { ^$co2_bin } | complete)
let gcc_run = (do { ^$gcc_bin } | complete)
if $co2_run.exit_code != $gcc_run.exit_code {
    print $"exit mismatch: co2cc ($co2_run.exit_code) vs gcc ($gcc_run.exit_code)"
    exit 1
}

# ---------- always_inline at -O0 ----------
let ai_src = ($test_dir | path join "always_inline.c")
let ai_co2_asm = ($test_dir | path join "ai_co2.s")
let ai_gcc_asm = ($test_dir | path join "ai_gcc.s")

let ai_co2_compile = (do { ^co2cc -O0 -S $ai_src -o $ai_co2_asm } | complete)
if $ai_co2_compile.exit_code != 0 {
    print $"co2cc -S -O0 failed: ($ai_co2_compile.stderr)"
    exit 1
}

let ai_gcc_compile = (do { ^gcc -O0 -S $ai_src -o $ai_gcc_asm } | complete)
if $ai_gcc_compile.exit_code != 0 {
    print $"gcc -S -O0 failed [oracle]: ($ai_gcc_compile.stderr)"
    exit 1
}

let ai_gcc_text = (open $ai_gcc_asm)
if $ai_gcc_text =~ 'call.*target' {
    print "gcc -O0 did not inline target despite always_inline [oracle broken?]"
    exit 1
}

let ai_co2_text = (open $ai_co2_asm)
if $ai_co2_text =~ 'call.*target' {
    print "co2cc -O0 did not inline target despite __attribute__((always_inline))"
    exit 1
}

exit 0
