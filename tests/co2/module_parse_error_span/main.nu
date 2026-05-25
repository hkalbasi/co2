#@ run-status: 0

def diff [a: string, b: string] {
    let f1 = (mktemp)
    let f2 = (mktemp)

    $a | save -f $f1
    $b | save -f $f2

    ^diff $f1 $f2
}

let test_dir = $env.CO2_TEST_DIR
cd $test_dir

let status = (do { ^co2cargo -q build } | complete)

# Compare Cargo stderr against the expected rendered diagnostic.
let actual = ($status.stderr | str trim)

let snapshot = (open stderr.snapshot | str trim)

if $actual != $snapshot {
    print "stderr mismatch!"
    print "--- GOT ---"
    print $actual
    print "--- EXPECTED ---"
    print $snapshot
    print "--- Diff ---"
    diff $snapshot $actual
    exit 1
}

exit 0
