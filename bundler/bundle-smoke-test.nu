#!/usr/bin/env nu

chmod +x ./co2-multicall.run

./co2-multicall.run install /home/testuser/bin
cp /test/stable-cargo /home/testuser/bin/cargo

$env.PATH = (["/home/testuser/bin"] ++ $env.PATH)

print -n "co2rustc version: "
co2rustc --version

print "Testing C compilation..."

"
#include <stdio.h>
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

print "Bundle test PASSED"
