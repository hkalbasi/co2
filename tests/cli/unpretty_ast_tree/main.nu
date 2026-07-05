#@ run-status: 0

use ./snapshot-utils.nu *

def validate-spans [test_dir: string, actual: string, name: string] {
    let dump = (mktemp)
    $actual | save -f $dump

    let status = (do { ^python validate_spans.py $test_dir $dump } | complete)
    if $status.exit_code != 0 {
        print $"span validation failed for ($name)"
        if ($status.stdout | str trim) != "" {
            print $status.stdout
        }
        if ($status.stderr | str trim) != "" {
            print $status.stderr
        }
        exit 1
    }
}

let test_dir = $env.CO2_TEST_DIR
cd $test_dir

let cases = [
    { name: use_and_rust, source: src/use_and_rust.rs, snapshot: use_and_rust.stdout.snapshot }
    { name: pack_and_types, source: src/pack_and_types.rs, snapshot: pack_and_types.stdout.snapshot }
    { name: storage_and_simple, source: src/storage_and_simple.rs, snapshot: storage_and_simple.stdout.snapshot }
    { name: macro_case, source: src/macro_case.rs, snapshot: macro_case.stdout.snapshot }
    { name: exprs, source: src/exprs.rs, snapshot: exprs.stdout.snapshot }
    { name: designators, source: src/designators.rs, snapshot: designators.stdout.snapshot }
    { name: holder_init, source: src/holder_init.rs, snapshot: holder_init.stdout.snapshot }
    { name: compound_literal, source: src/compound_literal.rs, snapshot: compound_literal.stdout.snapshot }
    { name: stmt_exprs, source: src/stmt_exprs.rs, snapshot: stmt_exprs.stdout.snapshot }
    { name: func_def, source: src/func_def.rs, snapshot: func_def.stdout.snapshot }
]

for case in $cases {
    let status = (do { ^co2rustc $case.source -Z unpretty=ast-tree } | complete)
    if $status.exit_code != 0 {
        print $"co2rustc -Z unpretty=ast-tree failed for ($case.name): ($status | to json -r)"
        exit 1
    }

    if ($status.stderr | str trim) != "" {
        print $"expected empty stderr for ($case.name), got: ($status.stderr)"
        exit 1
    }

    let actual = ($status.stdout | str trim)
    validate-spans $test_dir $actual $case.name
    let snapshot_path = $env.CO2_TEST_SOURCE_DIR | path join $case.snapshot
    assert-snapshot $case.name $actual $snapshot_path
}
