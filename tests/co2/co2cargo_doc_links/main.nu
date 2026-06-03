#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
cd ($test_dir | path join "demo")

let result = (do { ^co2cargo doc --no-deps --document-private-items } | complete)
if $result.exit_code != 0 {
    print $"expected successful co2cargo doc run, got: ($result | to json -r)"
    exit 2
}

let stderr = ($result.stderr | str trim)
if ($stderr | str contains "unresolved link") {
    print "FAIL: found unresolved link warnings in stderr"
    print $stderr
    exit 1
}

if ($stderr | str contains "broken_intra_doc_links") {
    print "FAIL: found broken_intra_doc_links warnings in stderr"
    print $stderr
    exit 1
}
