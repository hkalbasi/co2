#@ run-status: 0

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")

# simple C file
"int main(void) { return 0; }
int add(int a, int b) { return a + b; }
" | save -f test.c

# compile with -ftime-report
let result = (do { ^$co2cc -ftime-report -c test.c -o test.o } | complete)

if $result.exit_code != 0 {
    print $"-ftime-report compilation failed: ($result.stderr)"
    exit 1
}

let stderr = $result.stderr

# verify report contains all four phases
if ($stderr | str contains "Time report:") == false {
    print $"expected time report header but got: ($stderr)"
    exit 1
}

for phase in ["Preprocess", "Parse", "Lowering", "Codegen"] {
    if ($stderr | str contains $phase) == false {
        print $"expected phase ($phase) in time report but got: ($stderr)"
        exit 1
    }
}

# verify the object file exists
if ("test.o" | path exists) == false {
    print "co2cc -ftime-report did not produce object file"
    exit 1
}

# also verify without -ftime-report still works
let normal = (do { ^$co2cc -c test.c -o test_normal.o } | complete)
if $normal.exit_code != 0 {
    print $"normal compilation failed: ($normal.stderr)"
    exit 1
}
if ($normal.stderr | str contains "Time report:") == true {
    print "time report should not appear without -ftime-report flag"
    exit 1
}

exit 0
