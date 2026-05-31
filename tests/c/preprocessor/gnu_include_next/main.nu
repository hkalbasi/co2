#@ run-status: 0

let compile = (do { co2cc -I inc2 -I inc1 ok.c } | complete)
if $compile.exit_code != 0 {
    print $"compilation failed: ($compile.stderr)"
    exit 1
}

let compile2 = (do { co2cc -I inc2 -I inc1 err.c } | complete)
if $compile2.exit_code == 0 {
    print $"Compilation unexpectedly succeeded"
    print $"stdout: ($compile2.stdout)"
    print $"stderr: ($compile2.stderr)"
    exit 1
}

print "gnu_include_next test passed"
exit 0
