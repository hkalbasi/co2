#@ run-status: 0

# co2cc x.c -o /dev/null should succeed by linking through a temp file
# instead of trying to create sibling temp files next to /dev/null.
echo 'int main() { return 0; }' | save -f tiny.c

let compile = (do { co2cc tiny.c -o /dev/null } | complete)
if $compile.exit_code != 0 {
    print $"co2cc -o /dev/null failed: ($compile.stderr)"
    exit 1
}

let compile = (do { co2cc -c tiny.c -o /dev/null } | complete)
if $compile.exit_code != 0 {
    print $"co2cc -c -o /dev/null failed: ($compile.stderr)"
    exit 1
}

# /dev/null must remain a character device, not be replaced by a regular file.
let dev_null_type = (ls -l /dev/null | first | get type)
if $dev_null_type != "block device" and $dev_null_type != "char device" {
    print $"/dev/null was clobbered, type is now: ($dev_null_type)"
    exit 1
}

# Compilation must still run with -o /dev/null: a broken source should fail,
# not be silently skipped.
'int main() { return undefined_symbol_xyz; }' | save -f broken.c
let broken = (do { co2cc broken.c -o /dev/null } | complete)
if $broken.exit_code == 0 {
    print "co2cc -o /dev/null did not detect a compile error"
    exit 1
}
if ($broken.stderr | str contains "undefined_symbol_xyz") == false {
    print $"co2cc -o /dev/null missing expected diagnostic: ($broken.stderr)"
    exit 1
}

# A regular output path should still work and be runnable.
let real = (do { co2cc tiny.c -o tiny-real } | complete)
if $real.exit_code != 0 {
    print $"co2cc -o tiny-real failed: ($real.stderr)"
    exit 1
}
let run = (do { ./tiny-real } | complete)
if $run.exit_code != 0 {
    print $"tiny-real run failed with exit ($run.exit_code)"
    exit 1
}

exit 0
