#@ run-status: 0

# toybox pipes generated C through `cc -E -`: quoted includes must resolve
# relative to CWD like gcc, not relative to co2cc's stdin temp file.
let out = (do { '#include "lib/toyflags.h"' | co2cc -E - } | complete)
if $out.exit_code != 0 {
    print $"co2cc -E - failed: ($out.stderr)"
    exit 1
}
if ($out.stdout | str contains "flag_x") == false {
    print $"quoted stdin include did not expand vs CWD, got: ($out.stdout)"
    exit 1
}

exit 0
