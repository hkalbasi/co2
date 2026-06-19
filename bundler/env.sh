#!/bin/bash
# Source this script to set up the environment for co2 tools.
# CO2_CACHE_DIR can be pre-set; otherwise it is derived from this script's location.

export CO2_CACHE_DIR="${CO2_CACHE_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"

export LD_LIBRARY_PATH="$CO2_CACHE_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

# Tell rustc where its sysroot is
if [[ ! "$RUSTFLAGS" =~ "--sysroot" ]]; then
    export RUSTFLAGS="--sysroot=$CO2_CACHE_DIR $RUSTFLAGS"
fi

# Provide stdlib sources for miri if not already set
if [ -z "${MIRI_LIB_SRC}" ] && [ -d "$CO2_CACHE_DIR/lib/rustlib/src/rust/library" ]; then
    export MIRI_LIB_SRC="$CO2_CACHE_DIR/lib/rustlib/src/rust/library"
fi
