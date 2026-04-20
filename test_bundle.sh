#!/bin/bash
set -e

BUNDLE="target/co2-multicall.run"

if [ ! -f "$BUNDLE" ]; then
    echo "Bundle $BUNDLE not found. Running ./create_bundle.sh first..."
    ./create_bundle.sh
fi

echo "Testing bundle in bwrap container..."

bwrap \
  --ro-bind /usr /usr \
  --ro-bind /lib /lib \
  --ro-bind /lib64 /lib64 \
  --ro-bind /bin /bin \
  --dev /dev \
  --proc /proc \
  --tmpfs /tmp \
  --tmpfs /home \
  --setenv HOME /home/testuser \
  --dir /home/testuser \
  --bind "$BUNDLE" /test/co2-multicall.run \
  --chdir /test \
  /bin/bash << 'INNER_EOF'
    set -e
    chmod +x ./co2-multicall.run
    ./co2-multicall.run install /home/testuser/bin
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
    echo "Bundle test PASSED"
INNER_EOF
