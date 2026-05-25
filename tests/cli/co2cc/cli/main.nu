#@ run-status: 0

let compile = (do { 'int main() { return 5; }' | co2cc -x c - -o test-co2-from-stdin } | complete)
if $compile.exit_code != 0 {
    print $"co2cc failed: ($compile.stderr)"
    exit 1
}

let run = (do { ./test-co2-from-stdin } | complete)
if $run.exit_code != 5 {
    print $"app failed: ($run.stderr)"
    exit 1
}

echo 'int main() { return 0; }' | save -f tiny.c
let c_result = (do { co2cc -c tiny.c -o tiny.o } | complete)
if $c_result.exit_code != 0 {
    print $"co2cc -c failed: ($c_result.stderr)"
    exit 1
}

let file_result = (do { file tiny.o } | complete)
if ($file_result.stdout | str contains "relocatable") == false {
    print $"-c flag bug: expected object file but got: ($file_result.stdout | str trim)"
    exit 1
}

let gcc_o_result = (do { gcc tiny.o -o tiny-linked-by-gcc } | complete)
if $gcc_o_result.exit_code != 0 {
    print $"compiling co2cc object file using gcc failed: ($gcc_o_result.stderr)"
    exit 1
}

let co2cc_o_result = (do { co2cc tiny.o -o tiny-linked-by-co2cc } | complete)
if $co2cc_o_result.exit_code != 0 {
    print $"compiling co2cc object file using co2cc failed: ($co2cc_o_result.stderr)"
    exit 1
}

# compile and time O0 version
let o0_c = (do { co2cc -O0 sum_loop.c -o sum_loop_O0 } | complete)
if $o0_c.exit_code != 0 {
    print $"-O0 compilation failed: ($o0_c.stderr)"
    exit 1
}
let o0_start = (date now | into int)
let o0_run = (do { ./sum_loop_O0 } | complete)
let o0_end = (date now | into int)
let o0_time = $o0_end - $o0_start
if $o0_run.exit_code != 192 {
    print $"-O0 run: expected exit 192, got ($o0_run.exit_code)"
    exit 1
}

# compile and time O2 version
let o2_c = (do { co2cc -O2 sum_loop.c -o sum_loop_O2 } | complete)
if $o2_c.exit_code != 0 {
    print $"-O2 compilation failed: ($o2_c.stderr)"
    exit 1
}
let o2_start = (date now | into int)
let o2_run = (do { ./sum_loop_O2 } | complete)
let o2_end = (date now | into int)
let o2_time = $o2_end - $o2_start
if $o2_run.exit_code != 192 {
    print $"-O2 run: expected exit 192, got ($o2_run.exit_code)"
    exit 1
}

print $"  O0 time: ($o0_time) ns"
print $"  O2 time: ($o2_time) ns"

if $o2_time * 2 >= $o0_time {
    print $"optimization failed: O2 version ($o2_time) ns is not enough faster than O0 version ($o0_time) ns"
    exit 1
}

# verify object file without -g has no debug info
let no_debug_o = (do { readelf -S tiny.o } | complete)
if ($no_debug_o.stdout | str contains "debug_info") == true {
    print $"expected no debug info in object file without -g flag"
    exit 1
}

# test -g flag produces debug info in object file
let g_o_c = (do { co2cc -c -g sum_loop.c -o sum_loop_g.o } | complete)
if $g_o_c.exit_code != 0 {
    print $"-c -g compilation failed: ($g_o_c.stderr)"
    exit 1
}
let has_debug_o = (do { readelf -S sum_loop_g.o } | complete)
if ($has_debug_o.stdout | str contains "debug_info") == false {
    print $"-g flag bug: expected debug info in object file"
    exit 1
}

# compile -g full binary for timing comparison
let g_c = (do { co2cc -g sum_loop.c -o sum_loop_g } | complete)
if $g_c.exit_code != 0 {
    print $"-g compilation failed: ($g_c.stderr)"
    exit 1
}

# compare -g (debug, unoptimized) vs -O2 -g (debug, optimized)
let og_c = (do { co2cc -O2 -g sum_loop.c -o sum_loop_O2_g } | complete)
if $og_c.exit_code != 0 {
    print $"-O2 -g compilation failed: ($og_c.stderr)"
    exit 1
}

let g_start = (date now | into int)
let g_run = (do { ./sum_loop_g } | complete)
let g_end = (date now | into int)
let g_time = $g_end - $g_start
if $g_run.exit_code != 192 {
    print $"-g run: expected exit 192, got ($g_run.exit_code)"
    exit 1
}

let o2g_start = (date now | into int)
let o2g_run = (do { ./sum_loop_O2_g } | complete)
let o2g_end = (date now | into int)
let o2g_time = $o2g_end - $o2g_start
if $o2g_run.exit_code != 192 {
    print $"-O2 -g run: expected exit 192, got ($o2g_run.exit_code)"
    exit 1
}

print $"  -g time: ($g_time) ns"
print $"  -O2 -g time: ($o2g_time) ns"

if $o2g_time * 2 >= $g_time {
    print $"optimization failed: -O2 -g version ($o2g_time) ns is not enough faster than -g version ($g_time) ns"
    exit 1
}

exit 0
