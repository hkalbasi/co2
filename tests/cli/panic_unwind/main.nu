#@ run-status: 0

# Panic-catching coverage from a single co2 file:
# - co2rustc default (unwind): catch_unwind catches the panic -> exit 0
# - co2rustc -C panic=abort: the same panic aborts -> exit 134
# - co2cc (links panic=abort): the same panic aborts -> exit 134
#
# co2rustc rejects C `int main`, while co2cc's link needs a real `main`
# symbol, so the entry point is selected with `-DCO2CC` (accepted by co2cc's
# preprocessor args). The panicking/catching logic is identical in both
# branches. `write` (not `printf`) marks progress because stdio buffers
# are lost on abort.

# co2cc only accepts C inputs, so compile a copy under a .c name.
cp panic.co2 panic.c

# ---- co2rustc default: catch works, exit 0 ----
let unwind_compile = (do { ^co2rustc panic.co2 -o panic_unwind } | complete)
if $unwind_compile.exit_code != 0 {
    print $"co2rustc unwind compile failed: ($unwind_compile.stderr)"
    exit 1
}
let unwind_run = (do { ./panic_unwind } | complete)
if $unwind_run.exit_code != 0 {
    print $"unwind binary exit: expected 0, got ($unwind_run.exit_code): ($unwind_run | to json -r)"
    exit 1
}
if ($unwind_run.stdout | str contains "panicking") == false {
    print $"unwind binary never panicked: ($unwind_run | to json -r)"
    exit 1
}
if ($unwind_run.stdout | str contains "survived") == false {
    print $"unwind binary did not survive the caught panic: ($unwind_run | to json -r)"
    exit 1
}

# ---- co2rustc -C panic=abort: catch cannot act, abort ----
let abort_compile = (do { ^co2rustc -C panic=abort panic.co2 -o panic_abort } | complete)
if $abort_compile.exit_code != 0 {
    print $"co2rustc -C panic=abort compile failed: ($abort_compile.stderr)"
    exit 1
}
# SIGABRT death is reported by nushell as an uncatchable `core_dumped`
# error, so run under bash to get a regular exit code for `complete`.
let abort_run = (do { ^bash -c './panic_abort; code=$?; exit $code' } | complete)
if $abort_run.exit_code != 134 {
    print $"abort binary exit: expected 134, got ($abort_run.exit_code): ($abort_run | to json -r)"
    exit 1
}
if ($abort_run.stdout | str contains "panicking") == false {
    print $"abort binary never panicked: ($abort_run | to json -r)"
    exit 1
}
if ($abort_run.stdout | str contains "survived") {
    print $"abort binary unexpectedly survived panic: ($abort_run | to json -r)"
    exit 1
}
if ($abort_run.stderr | str contains "panicked") == false {
    print $"abort binary stderr missing panic report: ($abort_run | to json -r)"
    exit 1
}

# ---- co2cc: catch cannot act, abort ----
# (catch_unwind references _Unwind_Resume, so co2cc links the std stub here;
# the binary still aborts because co2cc links everything panic=abort.)
let c_compile = (do { ^co2cc -DCO2CC panic.c -o panic_c } | complete)
if $c_compile.exit_code != 0 {
    print $"co2cc compile failed: ($c_compile.stderr)"
    exit 1
}
let c_run = (do { ^bash -c './panic_c; code=$?; exit $code' } | complete)
if $c_run.exit_code != 134 {
    print $"co2cc binary exit: expected 134, got ($c_run.exit_code): ($c_run | to json -r)"
    exit 1
}
if ($c_run.stdout | str contains "panicking") == false {
    print $"co2cc binary never panicked: ($c_run | to json -r)"
    exit 1
}
if ($c_run.stdout | str contains "survived") {
    print $"co2cc binary unexpectedly survived panic: ($c_run | to json -r)"
    exit 1
}
if ($c_run.stderr | str contains "panicked") == false {
    print $"co2cc binary stderr missing panic report: ($c_run | to json -r)"
    exit 1
}

print "panic_unwind test passed"
exit 0
