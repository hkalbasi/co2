#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let co2cc_bin = ($test_dir | path join "archive_co2cc")
let gcc_bin = ($test_dir | path join "archive_gcc")
let co2cc_stripped = ($test_dir | path join "archive_co2cc_stripped")
let gcc_stripped = ($test_dir | path join "archive_gcc_stripped")

let sqlite_o = ($test_dir | path join "sqlite3.o")
let shell_o = ($test_dir | path join "shell.o")
let sqlite_gcc_o = ($test_dir | path join "sqlite3_gcc.o")
let shell_gcc_o = ($test_dir | path join "shell_gcc.o")
let co2cc_archive = ($test_dir | path join "libco2cc_fixture.a")
let gcc_archive = ($test_dir | path join "libgcc_fixture.a")

let compile_co2_sqlite = (do {
    ^co2cc -O2 -c ($test_dir | path join "sqlite3.c") -o $sqlite_o
} | complete)
if $compile_co2_sqlite.exit_code != 0 {
    print $"co2cc sqlite3 object compile failed: ($compile_co2_sqlite.stderr)"
    exit 1
}

let compile_co2_shell = (do {
    ^co2cc -O2 -c ($test_dir | path join "shell.c") -o $shell_o
} | complete)
if $compile_co2_shell.exit_code != 0 {
    print $"co2cc shell object compile failed: ($compile_co2_shell.stderr)"
    exit 1
}

let ar_co2 = (do {
    ^ar rc $co2cc_archive $sqlite_o
} | complete)
if $ar_co2.exit_code != 0 {
    print $"co2cc archive creation failed: ($ar_co2.stderr)"
    exit 1
}

let ranlib_co2 = (do { ^ranlib $co2cc_archive } | complete)
if $ranlib_co2.exit_code != 0 {
    print $"co2cc archive ranlib failed: ($ranlib_co2.stderr)"
    exit 1
}

let link_co2 = (do {
    ^co2cc -O2 -Wl,-E $shell_o $co2cc_archive -ldl -lpthread -lm -o $co2cc_bin
} | complete)
if $link_co2.exit_code != 0 {
    print $"co2cc archive link failed: ($link_co2.stderr)"
    exit 1
}

let compile_gcc_sqlite = (do {
    ^gcc -O2 -c ($test_dir | path join "sqlite3.c") -o $sqlite_gcc_o
} | complete)
if $compile_gcc_sqlite.exit_code != 0 {
    print $"gcc sqlite3 object compile failed: ($compile_gcc_sqlite.stderr)"
    exit 1
}

let compile_gcc_shell = (do {
    ^gcc -O2 -c ($test_dir | path join "shell.c") -o $shell_gcc_o
} | complete)
if $compile_gcc_shell.exit_code != 0 {
    print $"gcc shell object compile failed: ($compile_gcc_shell.stderr)"
    exit 1
}

let ar_gcc = (do {
    ^ar rc $gcc_archive $sqlite_gcc_o
} | complete)
if $ar_gcc.exit_code != 0 {
    print $"gcc archive creation failed: ($ar_gcc.stderr)"
    exit 1
}

let ranlib_gcc = (do { ^ranlib $gcc_archive } | complete)
if $ranlib_gcc.exit_code != 0 {
    print $"gcc archive ranlib failed: ($ranlib_gcc.stderr)"
    exit 1
}

let link_gcc = (do {
    ^gcc -O2 -Wl,-E $shell_gcc_o $gcc_archive -ldl -lpthread -lm -o $gcc_bin
} | complete)
if $link_gcc.exit_code != 0 {
    print $"gcc archive link failed: ($link_gcc.stderr)"
    exit 1
}

let co2cc_run = (do { ^$co2cc_bin } | complete)
if $co2cc_run.exit_code != 0 {
    print $"co2cc archive-linked binary exited with ($co2cc_run.exit_code): ($co2cc_run.stderr)"
    exit 1
}

let gcc_run = (do { ^$gcc_bin } | complete)
if $gcc_run.exit_code != 0 {
    print $"gcc archive-linked binary exited with ($gcc_run.exit_code): ($gcc_run.stderr)"
    exit 1
}

cp $co2cc_bin $co2cc_stripped
cp $gcc_bin $gcc_stripped

let strip_co2 = (do { ^strip $co2cc_stripped } | complete)
if $strip_co2.exit_code != 0 {
    print $"strip co2cc archive-linked binary failed: ($strip_co2.stderr)"
    exit 1
}

let strip_gcc = (do { ^strip $gcc_stripped } | complete)
if $strip_gcc.exit_code != 0 {
    print $"strip gcc archive-linked binary failed: ($strip_gcc.stderr)"
    exit 1
}

let co2cc_size = ((ls $co2cc_bin).0.size | into int)
let gcc_size = ((ls $gcc_bin).0.size | into int)
let co2cc_stripped_size = ((ls $co2cc_stripped).0.size | into int)
let gcc_stripped_size = ((ls $gcc_stripped).0.size | into int)

let co2_symbols = (do { ^nm -a $co2cc_bin } | complete)
if $co2_symbols.exit_code != 0 {
    print $"nm on co2cc binary failed: ($co2_symbols.stderr)"
    exit 1
}

print $"co2cc raw size: ($co2cc_size)"
print $"gcc raw size: ($gcc_size)"
print $"co2cc stripped size: ($co2cc_stripped_size)"
print $"gcc stripped size: ($gcc_stripped_size)"

let leaked_patterns = [
    "4core3net6parser"
    "4core3num7dec2flt"
    "4core3num7flt2dec"
    "panic_bounds_check"
    "panic_fmt"
    "slice_error_fail"
]

for pattern in $leaked_patterns {
    if (($co2_symbols.stdout | str contains $pattern) == true) {
        print $"co2cc archive-linked binary leaked unexpected core symbol pattern: ($pattern)"
        exit 1
    }
}

if $co2cc_stripped_size > ($gcc_stripped_size + 2048) {
    print "expected archive-linked co2cc binary to stay close to gcc after stripping"
    exit 1
}

exit 0
