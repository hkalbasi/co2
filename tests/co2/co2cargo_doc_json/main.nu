#@ run-status: 0

use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let expected_dir = $env.CO2_TEST_SOURCE_DIR
cd ($test_dir | path join "demo")

def run-doc [...extra_args: string] {
    let result = (do {
        with-env { RUSTDOCFLAGS: "-Z unstable-options --output-format json" } {
            ^co2cargo doc --no-deps ...$extra_args
        }
    } | complete)
    if $result.exit_code != 0 {
        print $"expected successful co2cargo doc json run, got: ($result | to json -r)"
        exit 2
    }
    return $result
}

def normalize-json [json_path: string] {
    let normalized = (do { ^python ($test_dir | path join "normalize_json.py") $json_path } | complete)
    if $normalized.exit_code != 0 {
        print $"failed to normalize rustdoc json output: ($normalized | to json -r)"
        exit 4
    }
    return ($normalized.stdout | str trim)
}

let json_path = ($test_dir | path join "demo" "target" "doc" "demo.json")

# --- Run 1: without --document-private-items ---
let doc1 = (run-doc)
let actual1 = (normalize-json $json_path)
assert-snapshot "rustdoc json" $actual1 ($expected_dir | path join "doc_json.snapshot")

let stderr_file1 = (mktemp)
$doc1.stderr | save -f $stderr_file1
let stderr_normalized1 = (do { ^python ($test_dir | path join "normalize_stderr.py") $stderr_file1 } | complete)
if $stderr_normalized1.exit_code != 0 {
    print $"failed to normalize stderr: ($stderr_normalized1 | to json -r)"
    exit 5
}
assert-snapshot "rustdoc stderr" $stderr_normalized1.stdout ($expected_dir | path join "doc_json.stderr.snapshot")

# --- Run 2: with --document-private-items ---
let doc2 = (run-doc "--document-private-items")
let actual2 = (normalize-json $json_path)
assert-snapshot "rustdoc json (private)" $actual2 ($expected_dir | path join "doc_json_private.snapshot")

let stderr_file2 = (mktemp)
$doc2.stderr | save -f $stderr_file2
let stderr_normalized2 = (do { ^python ($test_dir | path join "normalize_stderr.py") $stderr_file2 } | complete)
if $stderr_normalized2.exit_code != 0 {
    print $"failed to normalize stderr: ($stderr_normalized2 | to json -r)"
    exit 6
}
assert-snapshot "rustdoc stderr (private)" $stderr_normalized2.stdout ($expected_dir | path join "doc_json_private.stderr.snapshot")
