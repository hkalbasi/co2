#!/usr/bin/env nu

mkdir /opt/compiler-explorer
tar -x --zstd -f /opt/co2-ce.tar.zstd -C /opt/compiler-explorer

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
