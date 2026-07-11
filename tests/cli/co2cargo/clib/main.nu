# Test co2cargo test + cdylib/staticlib C linking for a co2 library crate (clib)
#
# 1. `co2cargo test` on `some-c-lib` (contains a `#[test]` in lib.co2).
# 2. `co2cargo build` and `co2cargo build --release` producing the
#    cdylib (.so) and staticlib (.a) for both profiles.
# 3. Link `palindrome_main.c` against both artifacts with gcc AND co2cc
#    (same flags for both — if co2cc rejects gcc flags that is a bug),
#    and verify the exported `is_palindrome` C function works.
#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let lib_dir = ($test_dir | path join "some-c-lib")
let lib_base = ($lib_dir | path join "target")
let c_main = ($test_dir | path join "palindrome_main.c")

cd $lib_dir

# Force a fresh build so the .so/.a reflect the current compiler.
rm -rf target

let status = (do { ^co2cargo test } | complete)
if $status.exit_code != 0 {
    print $"co2cargo test failed: ($status.stderr)"
    exit 1
}
if not ($status.stdout | str contains "1 passed") {
    print $"Test output did not show 1 passed: ($status.stdout)"
    exit 1
}

let build = (do { ^co2cargo build } | complete)
if $build.exit_code != 0 {
    print $"co2cargo build failed: ($build.stderr)"
    exit 2
}
let build_rel = (do { ^co2cargo build --release } | complete)
if $build_rel.exit_code != 0 {
    print $"co2cargo build --release failed: ($build_rel.stderr)"
    exit 3
}

let base_ld = ($env.LD_LIBRARY_PATH? | default "")

let profiles = [
    { dir: ($lib_base | path join "debug")   tag: "debug" },
    { dir: ($lib_base | path join "release") tag: "release" },
]
let compilers = [
    { bin: "gcc"   tag: "gcc" },
    { bin: "co2cc" tag: "co2" },
]

for compiler in $compilers {
    let cc = $compiler.bin
    let ctag = $compiler.tag
    for prof in $profiles {
        let ldir = $prof.dir
        let tag = $prof.tag

        let so = ($ldir | path join "libsome_c_lib.so")
        let a = ($ldir | path join "libsome_c_lib.a")
        if not (($so | path exists) and ($a | path exists)) {
            print $"expected cdylib and staticlib artifacts for ($tag): ($so) ($a)"
            exit 4
        }

        let ld_path = $"($ldir):($base_ld)"

        # shared (.so)
        ^($cc) $c_main -L $ldir -lsome_c_lib -o ($test_dir | path join $"($ctag)_so_($tag)")
        let r = (
            with-env { LD_LIBRARY_PATH: $ld_path } {
                do { ^($test_dir | path join $"($ctag)_so_($tag)") } | complete
            }
        )
        if $r.exit_code != 0 {
            print $"($cc) shared ($tag) run failed: ($r.stderr)"
            exit 5
        }
        if ($r.stdout | str trim) != "palindrome ok" {
            print $"($cc) shared ($tag) unexpected output: ($r.stdout)"
            exit 6
        }

        # static (.a)
        ^($cc) $c_main $a -o ($test_dir | path join $"($ctag)_a_($tag)")
        let r = (do { ^($test_dir | path join $"($ctag)_a_($tag)") } | complete)
        if $r.exit_code != 0 {
            print $"($cc) static ($tag) run failed: ($r.stderr)"
            exit 7
        }
        if ($r.stdout | str trim) != "palindrome ok" {
            print $"($cc) static ($tag) unexpected output: ($r.stdout)"
            exit 8
        }
    }
}

print "co2cargo clib cdylib/staticlib linking passed (debug + release, gcc + co2cc)"
exit 0
