def diff [a: string, b: string] {
    let f1 = (mktemp)
    let f2 = (mktemp)
    $a | save -f $f1
    $b | save -f $f2
    ^diff $f1 $f2
}

export def assert-snapshot [name: string, actual: string, snapshot_path: string] {
    if "CO2_UPDATE_SNAPSHOTS" in $env {
        print $"updating snapshot: ($snapshot_path)"
        $actual | save -f $snapshot_path
        return
    }
    let snapshot = (open $snapshot_path | str trim)
    if ($actual | str trim) != $snapshot {
        print $"FAIL: ($name) mismatch!"
        print "--- GOT ---"
        print $actual
        print "--- EXPECTED ---"
        print $snapshot
        print "--- Diff ---"
        diff $snapshot $actual
        exit 1
    }
}
