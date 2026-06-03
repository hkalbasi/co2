#@ run-status: 0

def diff [a: string, b: string] {
    let f1 = (mktemp)
    let f2 = (mktemp)

    $a | save -f $f1
    $b | save -f $f2

    ^diff $f1 $f2
}

let test_dir = $env.CO2_TEST_DIR
cd ($test_dir | path join "demo")

let doc = (do {
    with-env { RUSTDOCFLAGS: "-Z unstable-options --output-format json" } {
        ^co2cargo doc --no-deps
    }
} | complete)
if $doc.exit_code != 0 {
    print $"expected successful co2cargo doc json run, got: ($doc | to json -r)"
    exit 2
}

let json_path = ($test_dir | path join "demo" "target" "doc" "demo.json")
if (($json_path | path exists) == false) {
    print $"missing rustdoc json output: ($json_path)"
    exit 3
}

let normalized = (do { ^python ($test_dir | path join "normalize_json.py") $json_path } | complete)
if $normalized.exit_code != 0 {
    print $"failed to normalize rustdoc json output: ($normalized | to json -r)"
    exit 4
}

let actual = ($normalized.stdout | str trim)
let snapshot = (open ($test_dir | path join "doc_json.snapshot") | str trim)

if $actual != $snapshot {
    print "rustdoc json snapshot mismatch!"
    print "--- GOT ---"
    print $actual
    print "--- EXPECTED ---"
    print $snapshot
    print "--- Diff ---"
    diff $snapshot $actual
    exit 1
}

let stderr_file = (mktemp)
$doc.stderr | save -f $stderr_file
let stderr_normalized = (do { ^python ($test_dir | path join "normalize_stderr.py") $stderr_file } | complete)
if $stderr_normalized.exit_code != 0 {
    print $"failed to normalize stderr: ($stderr_normalized | to json -r)"
    exit 5
}

let stderr_expected = (open ($test_dir | path join "doc_json.stderr.snapshot"))
if $stderr_normalized.stdout != $stderr_expected {
    print "rustdoc stderr snapshot mismatch!"
    print "--- GOT ---"
    print $stderr_normalized.stdout
    print "--- EXPECTED ---"
    print $stderr_expected
    print "--- Diff ---"
    diff $stderr_expected $stderr_normalized.stdout
    exit 1
}
