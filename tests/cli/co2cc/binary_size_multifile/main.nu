#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let co2cc_bin = ($test_dir | path join "multifile_co2cc")
let gcc_bin = ($test_dir | path join "multifile_gcc")
let co2cc_stripped = ($test_dir | path join "multifile_co2cc_stripped")
let gcc_stripped = ($test_dir | path join "multifile_gcc_stripped")

let compile_co2 = (do {
    ^co2cc -O2 -lm ($test_dir | path join "main.c") ($test_dir | path join "helper.c") -o $co2cc_bin
} | complete)
if $compile_co2.exit_code != 0 {
    print $"co2cc -O2 multifile compile failed: ($compile_co2.stderr)"
    exit 1
}

let compile_gcc = (do {
    ^gcc -O2 -lm ($test_dir | path join "main.c") ($test_dir | path join "helper.c") -o $gcc_bin
} | complete)
if $compile_gcc.exit_code != 0 {
    print $"gcc -O2 multifile compile failed: ($compile_gcc.stderr)"
    exit 1
}

let co2cc_run = (do { ^$co2cc_bin } | complete)
if $co2cc_run.exit_code != 0 {
    print $"co2cc multifile binary exited with ($co2cc_run.exit_code): ($co2cc_run.stderr)"
    exit 1
}

let gcc_run = (do { ^$gcc_bin } | complete)
if $gcc_run.exit_code != 0 {
    print $"gcc multifile binary exited with ($gcc_run.exit_code): ($gcc_run.stderr)"
    exit 1
}

cp $co2cc_bin $co2cc_stripped
cp $gcc_bin $gcc_stripped

let strip_co2 = (do { ^strip $co2cc_stripped } | complete)
if $strip_co2.exit_code != 0 {
    print $"strip co2cc binary failed: ($strip_co2.stderr)"
    exit 1
}

let strip_gcc = (do { ^strip $gcc_stripped } | complete)
if $strip_gcc.exit_code != 0 {
    print $"strip gcc binary failed: ($strip_gcc.stderr)"
    exit 1
}

let co2cc_size = ((ls $co2cc_bin).0.size | into int)
let gcc_size = ((ls $gcc_bin).0.size | into int)
let co2cc_stripped_size = ((ls $co2cc_stripped).0.size | into int)
let gcc_stripped_size = ((ls $gcc_stripped).0.size | into int)

let co2_sections = (do { ^readelf -SW $co2cc_bin } | complete)
if $co2_sections.exit_code != 0 {
    print $"readelf on co2cc binary failed: ($co2_sections.stderr)"
    exit 1
}

let gcc_sections = (do { ^readelf -SW $gcc_bin } | complete)
if $gcc_sections.exit_code != 0 {
    print $"readelf on gcc binary failed: ($gcc_sections.stderr)"
    exit 1
}

print $"co2cc raw size: ($co2cc_size)"
print $"gcc raw size: ($gcc_size)"
print $"co2cc stripped size: ($co2cc_stripped_size)"
print $"gcc stripped size: ($gcc_stripped_size)"

if (($co2_sections.stdout | str contains ".debug_info") == false) {
    print "expected multifile co2cc -O2 binary to contain .debug_info for this reproducer"
    exit 1
}

if (($gcc_sections.stdout | str contains ".debug_info") == true) {
    print "expected gcc -O2 multifile binary to not contain .debug_info"
    exit 1
}

if $co2cc_size <= $gcc_size {
    print "expected multifile co2cc -O2 binary to be larger than gcc -O2 binary before stripping"
    exit 1
}

if $co2cc_stripped_size >= $gcc_stripped_size {
    print "expected stripping to remove the multifile co2cc size inflation"
    exit 1
}

exit 0
