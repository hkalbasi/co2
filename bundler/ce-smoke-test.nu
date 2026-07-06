#!/usr/bin/env nu

# Version of co2 tools embedded in the bundle.
let expected_version = $env.CO2_EXPECTED_VERSION? | default "unknown"

mkdir /opt/compiler-explorer
tar -x --zstd -f /opt/co2-ce.tar.zstd -C /opt/compiler-explorer

# Verify version strings across all applets
# co2miri and co2fmt use the CO2_VERSION format
for applet in ["co2miri", "co2fmt"] {
    let bin = $"/opt/compiler-explorer/bin/($applet)"
    let ver = (do { ^$bin --version } | complete)
    if $ver.exit_code != 0 {
        print $"FAIL: ($applet) --version exit code ($ver.exit_code)"
        exit 1
    }
    let expected = $"($applet) ($expected_version)"
    if ($ver.stdout | str trim) != $expected {
        print $"FAIL: ($applet) --version expected '($expected)', got '($ver.stdout | str trim)'"
        exit 1
    }
    print $"($applet) version OK"
}

# co2rustc shows co2rustc header + embedded rustc version
let bin = "/opt/compiler-explorer/bin/co2rustc"
let ver = (do { ^$bin --version } | complete)
if $ver.exit_code != 0 {
    print $"FAIL: co2rustc --version exit code ($ver.exit_code)"
    exit 1
}
# Did you address the rustversion problem?
if ($ver.stdout | str contains "co2rustc") == true {
    print $"FAIL: co2rustc --version had co2rustc header, got: ($ver.stdout)"
    exit 1
}
if ($ver.stdout | str contains "rustc") == false {
    print $"FAIL: co2rustc --version expected rustc version, got: ($ver.stdout)"
    exit 1
}
print "co2rustc version OK"

# co2cc shows co2cc header + rustc version + clang version (needed for meson)
let bin = "/opt/compiler-explorer/bin/co2cc"
let ver = (do { ^$bin --version } | complete)
if $ver.exit_code != 0 {
    print $"FAIL: co2cc --version exit code ($ver.exit_code)"
    exit 1
}
if ($ver.stdout | str contains "co2cc") == false {
    print $"FAIL: co2cc --version missing co2cc header, got: ($ver.stdout)"
    exit 1
}
if ($ver.stdout | str contains "rustc") == false {
    print $"FAIL: co2cc --version missing rustc version, got: ($ver.stdout)"
    exit 1
}
if ($ver.stdout | str contains "clang version:") == false {
    print $"FAIL: co2cc --version missing clang version, got: ($ver.stdout)"
    exit 1
}
print "co2cc version OK"

print "Testing C compilation..."

# Write test C file
"
#include <stdio.h>
int main() {
    printf(\"Hello from co2cc on CE!\\n\");
    return 0;
}
" | save --force /tmp/hello.c

/opt/compiler-explorer/bin/co2cc /tmp/hello.c -o /tmp/hello
/tmp/hello

print "Testing co2 compilation..."

"
use libc::puts;

fn main() {
    puts(\"Hello from co2 on CE!\");
}
" | save --force /tmp/hello.co2

/opt/compiler-explorer/bin/co2rustc /tmp/hello.co2 -o /tmp/hello-co2
/tmp/hello-co2

print "Testing co2 with miri..."

"
use std::process::exit;

fn main() {
    i32 v = 42;
    i32* ptr = &v;
    if (*ptr != 42) {
        exit(1);
    }
}
" | save --force /tmp/hello-miri.co2
"#![language(co2)]
" | save --force /tmp/hello-miri.rs

/opt/compiler-explorer/bin/co2miri --sysroot /opt/compiler-explorer/miri-sysroot /tmp/hello-miri.rs

print "Testing co2fmt..."

"
#include <stdio.h>
int main(void) {
    printf(\"Hello from co2fmt on CE!\\n\");
    return 0;
}
" | save --force /tmp/hello-fmt.c

/opt/compiler-explorer/bin/co2fmt --check /tmp/hello-fmt.c

print "CE bundle test PASSED"
