# Test `cc` crate + `co2cc` linking in a co2cargo project.
#
# `proj/build.rs` uses the `cc` crate with `.compiler("co2cc")` to
# compile `src/native/add.c`, and `src/main.co2` calls the resulting
# `add_from_c` symbol. A successful `co2cargo run` proves the
# cc-built staticlib linked into the co2 binary.
#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let proj_dir = ($test_dir | path join "proj")

cd $proj_dir

# Force a fresh build so the C object reflects the current co2cc.
rm -rf target

let build = (do { ^co2cargo build } | complete)
if $build.exit_code != 0 {
    print $"co2cargo build failed: ($build.stderr)"
    exit 1
}

let run = (do { ^co2cargo run -q } | complete)
if $run.exit_code != 0 {
    print $"co2cargo run failed: ($run.stderr)"
    exit 2
}
if ($run.stdout | str trim) != "sum=42" {
    print $"unexpected output: ($run.stdout)"
    exit 3
}

# Prove the `cc` crate path ran (not a checked-in artifact).
let libs = (glob target/debug/build/proj/*/out/libadd.a)
if ($libs | is-empty) {
    print "cc build output libadd.a not found"
    exit 4
}

print "co2cargo cc + co2cc linking passed"
exit 0
