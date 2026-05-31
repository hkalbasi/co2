#!/bin/bash
set -e

checkpoint() {
    local now
    now=$(date +%s.%N)

    if [[ -n "${__checkpoint_last:-}" ]]; then
        local delta
        delta=$(awk "BEGIN { printf \"%.3f\", $now - $__checkpoint_last }")
        printf '[+%ss] %s\n' "$delta" "$*"
    else
        printf '[start] %s\n' "$*"
    fi

    __checkpoint_last=$now
}

checkpoint Starting

# 1. Build co2-multicall
cargo build -p co2-multicall --release

checkpoint Build successfully

# 2. Prepare payload directory
PAYLOAD_DIR=$(mktemp -d)
mkdir -p "$PAYLOAD_DIR/bin"
mkdir -p "$PAYLOAD_DIR/lib"

cp target/release/co2-multicall "$PAYLOAD_DIR/bin/"

checkpoint Prepared payload dir

# 3. Collect libs
SYSROOT=$(rustc --print sysroot)
mkdir -p "$PAYLOAD_DIR/lib"
cp "$SYSROOT"/lib/librustc_driver-*.so "$PAYLOAD_DIR/lib/"
cp "$SYSROOT"/lib/libLLVM*.so* "$PAYLOAD_DIR/lib/" || true

checkpoint Collected libs

# Include ONLY stdlib for compilation in a way that rustc recognizes as sysroot
TARGET_LIB_DIR="$PAYLOAD_DIR/lib/rustlib/x86_64-unknown-linux-gnu/lib"
mkdir -p "$TARGET_LIB_DIR"

# List of essential stdlib crates
STD_CRATES=(std test getopts unicode_width core alloc panic_unwind panic_abort unwind libc compiler_builtins std_detect rustc_std_workspace_core rustc_std_workspace_alloc rustc_std_workspace_std miniz_oxide addr2line object gimli hashbrown adler2 memchr rustc_demangle cfg_if)

for crate in "${STD_CRATES[@]}"; do
    # Copy both .rlib and .rmeta if they exist
    cp "$SYSROOT"/lib/rustlib/x86_64-unknown-linux-gnu/lib/lib${crate}-*.{rlib,rmeta} "$TARGET_LIB_DIR/" 2>/dev/null || true
done

# Include self-contained linker components if they exist
if [ -d "$SYSROOT/lib/rustlib/x86_64-unknown-linux-gnu/bin" ]; then
    mkdir -p "$PAYLOAD_DIR/lib/rustlib/x86_64-unknown-linux-gnu/bin"
    cp -r "$SYSROOT"/lib/rustlib/x86_64-unknown-linux-gnu/bin/* "$PAYLOAD_DIR/lib/rustlib/x86_64-unknown-linux-gnu/bin/"
fi

# Include rust stdlib source for miri (enables `co2cargo miri run` without MIRI_LIB_SRC)
if [ -d "$SYSROOT/lib/rustlib/src/rust/library" ]; then
    mkdir -p "$PAYLOAD_DIR/lib/rustlib/src/rust"
    cp -r "$SYSROOT/lib/rustlib/src/rust/library" "$PAYLOAD_DIR/lib/rustlib/src/rust/"
fi

checkpoint Finished payload dir

# 4. Create tarball (use --zstd flag to enable zstd compression)
COMPRESS_FLAG="-z"
if [ "${1:-}" = "--zstd" ]; then
    COMPRESS_FLAG="--zstd"
    echo "Using zstd compression"
else
    echo "Using gzip compression"
fi

TARBALL=$(mktemp)
tar -C "$PAYLOAD_DIR" -c "$COMPRESS_FLAG" -f "$TARBALL" .

checkpoint Created tarball

HASH=$(sha256sum "$TARBALL" | cut -d' ' -f1)

checkpoint Evaluated hash of tarball

# 5. Create the self-extracting script
OUT_FILE="target/co2-multicall.run"
mkdir -p target

cat << EOF > "$OUT_FILE"
#!/bin/bash
set -e

HASH="$HASH"
CACHE_DIR="\$HOME/.cache/co2/\$HASH"

if [ ! -d "\$CACHE_DIR" ]; then
    # Find payload start - only when extracting
    PAYLOAD_LINE=\$(grep -a -n "^__PAYLOAD_BELOW__" "\$0" | head -n 1 | cut -d: -f1)
    PAYLOAD_START=\$((PAYLOAD_LINE + 1))
    mkdir -p "\$CACHE_DIR"
    tail -n +\$PAYLOAD_START "\$0" | tar -x $COMPRESS_FLAG -C "\$CACHE_DIR"
fi

# Multicall dispatch: use the name this script was called as
ARG0="\$0"
export LD_LIBRARY_PATH="\$CACHE_DIR/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
export CO2_RUN_SCRIPT="\$(readlink -f "\$0")"
# Tell rustc where its sysroot is
if [[ ! "\$RUSTFLAGS" =~ "--sysroot" ]]; then
    export RUSTFLAGS="--sysroot=\$CACHE_DIR \$RUSTFLAGS"
fi
# Provide stdlib sources for miri if not already set
if [ -z "\${MIRI_LIB_SRC}" ] && [ -d "\$CACHE_DIR/lib/rustlib/src/rust/library" ]; then
    export MIRI_LIB_SRC="\$CACHE_DIR/lib/rustlib/src/rust/library"
fi

# Forward all arguments with preserved arg0
exec -a "\$ARG0" "\$CACHE_DIR/bin/co2-multicall" "\$@"
EOF

checkpoint Created self extracting script

# Append the binary data
echo "__PAYLOAD_BELOW__" >> "$OUT_FILE"
cat "$TARBALL" >> "$OUT_FILE"

chmod +x "$OUT_FILE"

# Cleanup
rm -rf "$PAYLOAD_DIR"
rm "$TARBALL"

checkpoint Finished

echo "Created $OUT_FILE"
