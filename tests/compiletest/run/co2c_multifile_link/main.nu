#@ run-status: 0

let compile = (do { ^co2c -lm sqlite3.c shell.c -o app } | complete)
if $compile.exit_code != 0 {
    print $"co2c failed: ($compile.stderr)"
    exit 1
}

let run = (do { ./app } | complete)
if $run.exit_code != 0 {
    print $"app failed: ($run.stderr)"
    exit 2
}

exit 0
