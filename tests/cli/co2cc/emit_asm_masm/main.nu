#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let source = ($test_dir | path join "trivial.c")
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cc" "emit_asm_masm")

def test-flavor [flavor: string, snapshot_file: string] {
    let asm_file = ($test_dir | path join $"co2cc_trivial_($flavor).s")
    let bin_file = ($test_dir | path join $"co2cc_trivial_($flavor)")

    # compile to assembly
    let compile = (do { co2cc -S -O2 $"-masm=($flavor)" $source -o $asm_file } | complete)
    if $compile.exit_code != 0 {
        print $"co2cc -S -O2 -masm=($flavor) failed: ($compile.stderr)"
        exit 1
    }

    # verify assembly output is non-empty
    let asm_size = ((ls $asm_file).0.size | into int)
    if $asm_size <= 10 {
        print $"co2cc -S -O2 -masm=($flavor) produced suspiciously small assembly: ($asm_size) bytes"
        exit 1
    }

    # snapshot check: compare normalized assembly (skip .file and .ident lines)
    let asm_text = (open $asm_file)
    let normalize = {|s|
        $s | lines | where {|line|
            let t = ($line | str trim)
            ($t | str starts-with ".file") == false and ($t | str starts-with ".ident") == false
        } | str join "\n"
    }
    let actual = (do $normalize $asm_text)
    assert-snapshot $"-masm=($flavor) assembly" $actual ($expected_dir | path join $snapshot_file)

    # assemble and link, then verify they produce the expected exit code
    let link = (do { gcc $asm_file -o $bin_file } | complete)
    if $link.exit_code != 0 {
        print $"assembling/linking -masm=($flavor) asm failed: ($link.stderr)"
        exit 1
    }

    let run = (do { ^$bin_file } | complete)
    if $run.exit_code != 42 {
        print $"expected exit 42 for -masm=($flavor), got ($run.exit_code)"
        exit 1
    }
}

test-flavor "att" "att.snapshot"
test-flavor "intel" "intel.snapshot"

exit 0
