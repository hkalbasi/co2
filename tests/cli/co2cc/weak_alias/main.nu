#@ run-status: 0

# GNU weak + alias extension test. Builds a binary from several translation
# units that exercise:
#   * a weak alias (`crypt_r` -> `__crypt_r`),
#   * a weak symbol overridden by a strong definition in another TU (`weak_only`),
#   * an ordinary strong symbol (`strong_sym`).

let test_dir = $env.CO2_TEST_DIR

let sources = [
    ($test_dir | path join "target.c")
    ($test_dir | path join "weak.c")
    ($test_dir | path join "override.c")
    ($test_dir | path join "main.c")
]

let out = ($test_dir | path join "weak_alias_app")

let compile = (do {
    ^co2cc ...$sources -o $out
} | complete)
if $compile.exit_code != 0 {
    print $"co2cc failed: ($compile.stderr)"
    exit 1
}

let run = (do { ^$out } | complete)
if $run.exit_code != 0 {
    print $"binary exited with ($run.exit_code), expected 0: ($run.stderr)"
    exit 2
}

exit 0
