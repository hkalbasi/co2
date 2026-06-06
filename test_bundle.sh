#!/bin/bash
set -e

BUNDLE="target/co2-multicall.run"
STABLE_CARGO=$(rustup which --toolchain stable cargo)

echo "Rebuilding bundle..."
./create_bundle.sh --zstd

echo "Testing bundle in bwrap container..."

bwrap \
  --ro-bind /usr /usr \
  --ro-bind /lib /lib \
  --ro-bind /etc /etc \
  --ro-bind /lib64 /lib64 \
  --ro-bind /bin /bin \
  --dev /dev \
  --proc /proc \
  --tmpfs /tmp \
  --tmpfs /home \
  --setenv HOME /home/testuser \
  --dir /home/testuser \
  --bind "$BUNDLE" /test/co2-multicall.run \
  --ro-bind "$STABLE_CARGO" /test/stable-cargo \
  --chdir /test \
  /bin/bash << 'INNER_EOF'
    set -e
    chmod +x ./co2-multicall.run
    ./co2-multicall.run install /home/testuser/bin
    cp /test/stable-cargo /home/testuser/bin/cargo
    export PATH="/home/testuser/bin:$PATH"

    echo -n "co2rustc version: "
    co2rustc --version

    echo "Testing C compilation..."
    cat << 'EOF' > hello.c
#include <stdio.h>
int main() {
    printf("Hello from co2cc isolated!\n");
    return 0;
}
EOF
    co2cc hello.c -o hello
    ./hello

    echo "Testing co2cargo test..."
    mkdir demo
    cd demo
    cat << 'EOF' > Cargo.toml
[package]
name = "demo"
version = "0.1.0"
edition = "2024"

[dependencies]
EOF
    mkdir src
    cat << 'EOF' > src/lib.rs
#![language(co2)]
EOF
    cat << 'EOF' > src/lib.co2
mod nested;

#[test]
fn smoke_test() {
    helper();
}

fn helper() {}
EOF
    cat << 'EOF' > src/nested.co2
#[test]
fn smoke_test() {
    super::helper();
}
EOF
    co2cargo test
    cd ..
    echo "Bundle test PASSED"
INNER_EOF
