#@ run-status: 0

let compile = (do { ^co2rustc main.rs } | complete)
if $compile.exit_code != 0 {
    print $"co2rustc failed: ($compile.stderr)"
    exit 1
}

exit 0
