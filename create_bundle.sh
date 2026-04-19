#!/bin/bash
set -e

# 1. Build co2-multicall
cargo build -p co2-multicall --release

# 2. Prepare payload directory
PAYLOAD_DIR=$(mktemp -d)
mkdir -p "$PAYLOAD_DIR/bin"
mkdir -p "$PAYLOAD_DIR/lib"

cp target/release/co2-multicall "$PAYLOAD_DIR/bin/"

# 3. Collect libs
SYSROOT=$(rustc --print sysroot)
cp "$SYSROOT"/lib/librustc_driver-*.so "$PAYLOAD_DIR/lib/"
cp "$SYSROOT"/lib/libLLVM*.so* "$PAYLOAD_DIR/lib/"

# 4. Create tarball
TARBALL=$(mktemp)
tar -C "$PAYLOAD_DIR" -czf "$TARBALL" .

# 5. Create the self-extracting script
OUT_FILE="target/co2-multicall.run"
mkdir -p target

cat << 'EOF' > "$OUT_FILE"
#!/bin/bash
set -e

# Find payload start
PAYLOAD_LINE=$(grep -a -n "^__PAYLOAD_BELOW__" "$0" | head -n 1 | cut -d: -f1)
PAYLOAD_START=$((PAYLOAD_LINE + 1))

# Hash the payload part for the cache directory
HASH=$(tail -n +$PAYLOAD_START "$0" | sha256sum | cut -d' ' -f1)
CACHE_DIR="$HOME/.cache/co2/$HASH"

if [ ! -d "$CACHE_DIR" ]; then
    mkdir -p "$CACHE_DIR"
    tail -n +$PAYLOAD_START "$0" | tar -xz -C "$CACHE_DIR"
fi

# Multicall dispatch: use the name this script was called as
ARG0="$0"
export LD_LIBRARY_PATH="$CACHE_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export CO2_RUN_SCRIPT="$(readlink -f "$0")"

# Forward all arguments with preserved arg0
exec -a "$ARG0" "$CACHE_DIR/bin/co2-multicall" "$@"
EOF

# Append the binary data
echo "__PAYLOAD_BELOW__" >> "$OUT_FILE"
cat "$TARBALL" >> "$OUT_FILE"

chmod +x "$OUT_FILE"

# Cleanup
rm -rf "$PAYLOAD_DIR"
rm "$TARBALL"

echo "Created $OUT_FILE"
