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

exit 0
