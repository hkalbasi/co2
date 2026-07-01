#@ run-status: 0

use snapshot-utils.nu ["assert-snapshot"]

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")
let src_dir = $env.CO2_TEST_SOURCE_DIR

def normalize-dep []: string -> string {
    $in | str replace -a $env.CO2_TEST_DIR "\$DIR"
}

def extract-dep-files [d_file: string] {
    let lines = (open $d_file | lines)
    let flat = ($lines | each {|line|
        $line | str replace -r '\\\s*$' '' | str trim
    } | str join ' ')
    let colon_idx = ($flat | str index-of ':')
    let after_target = if $colon_idx != -1 {
        $flat | str substring ($colon_idx + 1).. | str trim
    } else {
        $flat | str trim
    }
    $after_target | split words | where ($it | str contains '.') | each {|f| $f | path basename} | uniq | sort
}

def assert-deps-match-gcc [d_file: string, label: string, gcc_d_file: string] {
    let co2_files = (extract-dep-files $d_file)
    let gcc_files = (extract-dep-files $gcc_d_file)
    for f in $co2_files {
        if ($f not-in $gcc_files) {
            print $"FAIL: ($label) dep '($f)' not found in gcc output"
            print $"co2cc deps: ($co2_files)"
            print $"gcc deps: ($gcc_files)"
            exit 1
        }
    }
}

# ---- -MD basic ----
print "test: -MD basic"
do { ^$co2cc -MD -c main.c -o main.o } | ignore
let nd1 = (open main.d | normalize-dep)
assert-snapshot "basic" $nd1 ($src_dir | path join "basic.d.snapshot")
do { gcc -MD -c main.c -o gcc_check.o } | ignore
assert-deps-match-gcc "main.d" "basic" "gcc_check.d"
rm main.o main.d gcc_check.o gcc_check.d

# ---- -MD -MP ----
print "test: -MD -MP"
do { ^$co2cc -MD -MP -c main.c -o main.o } | ignore
let nd2 = (open main.d | normalize-dep)
assert-snapshot "with_mp" $nd2 ($src_dir | path join "with_mp.d.snapshot")
do { gcc -MD -MP -c main.c -o gcc_mp_check.o } | ignore
assert-deps-match-gcc "main.d" "with_mp" "gcc_mp_check.d"
rm main.o main.d gcc_mp_check.o gcc_mp_check.d

# ---- -MD -MF ----
print "test: -MD -MF"
do { ^$co2cc -MD -MF out.d -c main.c -o main.o } | ignore
let nd3 = (open out.d | normalize-dep)
assert-snapshot "mf" $nd3 ($src_dir | path join "mf.d.snapshot")
do { gcc -MD -MF gcc_out.d -c main.c -o gcc_mf_check.o } | ignore
assert-deps-match-gcc "out.d" "mf" "gcc_out.d"
rm main.o out.d gcc_mf_check.o gcc_out.d

# ---- -MD -MT ----
print "test: -MD -MT"
do { ^$co2cc -MD -MT "my_target.o" -c main.c -o main.o } | ignore
let nd4 = (open main.d | normalize-dep)
assert-snapshot "mt" $nd4 ($src_dir | path join "mt.d.snapshot")
do { gcc -MD -MT "my_target.o" -c main.c -o gcc_mt_check.o } | ignore
assert-deps-match-gcc "main.d" "mt" "gcc_mt_check.d"
rm main.o main.d gcc_mt_check.o gcc_mt_check.d

# ---- -MD -MP -MF -MT combined ----
print "test: combined"
do { ^$co2cc -MD -MP -MF out.d -MT custom.o -c main.c -o main.o } | ignore
let nd5 = (open out.d | normalize-dep)
assert-snapshot "combined" $nd5 ($src_dir | path join "combined.d.snapshot")
do { gcc -MD -MP -MF gcc_out.d -MT custom.o -c main.c -o gcc_combined_check.o } | ignore
assert-deps-match-gcc "out.d" "combined" "gcc_out.d"
rm main.o out.d gcc_combined_check.o gcc_out.d

# ---- -MD -MQ ----
print "test: -MD -MQ"
do { ^$co2cc -MD -MQ "obj/foo.o" -c main.c -o main.o } | ignore
let nd6 = (open main.d | normalize-dep)
if ($nd6 | str starts-with "obj/foo.o:") == false {
    print $"FAIL: -MD -MQ wrong target, got: ($nd6)"; exit 1
}
do { gcc -MD -MQ "obj/foo.o" -c main.c -o gcc_mq_check.o } | ignore
assert-deps-match-gcc "main.d" "mq" "gcc_mq_check.d"
rm main.o main.d gcc_mq_check.o gcc_mq_check.d

# ---- no -MD: no .d file ----
print "test: no -MD (no .d)"
do { ^$co2cc -c main.c -o main.o } | ignore
if ("main.d" | path exists) == true { print "FAIL: plain -c produced main.d"; exit 1 }
rm main.o

exit 0
