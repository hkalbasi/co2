#!/usr/bin/env nu

# Version of co2 tools embedded in the bundle.
let expected_version = $env.CO2_EXPECTED_VERSION? | default "unknown"

chmod +x ./co2-multicall.run

./co2-multicall.run install /home/testuser/bin
cp /test/stable-cargo /home/testuser/bin/cargo

$env.PATH = (["/home/testuser/bin"] ++ $env.PATH)

# Verify version strings across all applets
# co2cargo and co2fmt use the CO2_VERSION format
for applet in ["co2cargo", "co2fmt"] {
    let ver = (do { ^$applet --version } | complete)
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
let ver = (do { ^co2rustc --version } | complete)
if $ver.exit_code != 0 {
    print $"FAIL: co2rustc --version exit code ($ver.exit_code)"
    exit 1
}
if ($ver.stdout | str contains "co2rustc") == false {
    print $"FAIL: co2rustc --version missing co2rustc header, got: ($ver.stdout)"
    exit 1
}
if ($ver.stdout | str contains "rustc") == false {
    print $"FAIL: co2rustc --version expected rustc version, got: ($ver.stdout)"
    exit 1
}
print "co2rustc version OK"

# co2cc shows co2cc header + rustc version + LLVM version
let ver = (do { ^co2cc --version } | complete)
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
# Presenting LLVM version as clang version is needed for meson.
if ($ver.stdout | str contains "clang version:") == false {
    print $"FAIL: co2cc --version missing clang version, got: ($ver.stdout)"
    exit 1
}
print "co2cc version OK"

print "Testing C compilation..."

"
#include <stdio.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <sys/param.h>

int main() {
    printf(\"Hello from co2cc isolated!\\n\");
    return 0;
}
" | save --force hello.c

co2cc hello.c -o hello
./hello

print "Testing co2cargo test..."

mkdir demo
cd demo

"
[package]
name = \"demo\"
version = \"0.1.0\"
edition = \"2024\"

[dependencies]
" | save --force Cargo.toml

mkdir src

"#![language(co2)]
" | save --force src/lib.rs

"
mod nested;

#[test]
fn smoke_test() {
    helper();
}

fn helper() {}
" | save --force src/lib.co2

"
#[test]
fn smoke_test() {
    super::helper();
}
" | save --force src/nested.co2

co2cargo test

co2rustc src/lib.rs --edition 2024 -C instrument-coverage --test

cd ..

print "Testing co2fmt..."

"
#include <stdio.h>
int main(void) {
    printf(\"Hello, world!\\n\");
    return 0;
}
" | save --force hello.c

co2fmt --check hello.c

print "Bundle test PASSED"
