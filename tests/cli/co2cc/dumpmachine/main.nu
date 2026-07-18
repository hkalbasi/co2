#@ run-status: 0

use snapshot-utils.nu ["assert-snapshot"]

# co2cc -dumpmachine should print the target machine triple, like gcc/clang.
let dm = (do { co2cc -dumpmachine } | complete)
if $dm.exit_code != 0 {
    print $"co2cc -dumpmachine failed: ($dm.stderr)"
    exit 1
}

let triple = ($dm.stdout | str trim)
let snap_path = ($env.CO2_TEST_SOURCE_DIR | path join "triple.snapshot")
assert-snapshot "dumpmachine triple" $triple $snap_path

exit 0
