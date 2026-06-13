#!/usr/bin/env nu

# Helper to print progress with timing, mimicking the bash checkpoint function.
def --env checkpoint [message: string] {
    let now = (date now | into int)  # nanoseconds since epoch
    if ($env.__CHECKPOINT_LAST? | is-not-empty) {
        let last = ($env.__CHECKPOINT_LAST | into int)
        let delta_ns = $now - $last
        let delta_sec = ($delta_ns / 1_000_000_000 | math round --precision 3)
        print $"[+($delta_sec)s] ($message)"
    } else {
        print $"[start] ($message)"
    }
    $env.__CHECKPOINT_LAST = ($now | into string)
}

def main [--zstd] {

    checkpoint "Starting"

    # 1. Build co2-multicall
    cargo build -p co2-multicall --release

    checkpoint "Build successfully"

    # 2. Prepare payload directory
    let payload_dir = (mktemp -d)
    mkdir ($payload_dir | path join "bin")
    mkdir ($payload_dir | path join "lib")

    cp target/release/co2-multicall ($payload_dir | path join "bin" "co2-multicall")

    checkpoint "Prepared payload dir"

    # 3. Collect libraries
    let sysroot = (rustc --print sysroot | str trim)

    # librustc_driver
    let driver_sos = (glob $"($sysroot)/lib/librustc_driver-*.so")
    for f in $driver_sos {
        cp $f ($payload_dir | path join "lib")
    }

    # LLVM (ignore missing files)
    let llvm_sos = (glob $"($sysroot)/lib/libLLVM*.so*")
    for f in $llvm_sos {
        cp $f ($payload_dir | path join "lib")
    }

    checkpoint "Collected libs"

    # Strip debug info from shared libraries
    let so_files = (glob $"($payload_dir)/lib/*.so*")
    for lib in $so_files {
        try { strip --strip-debug $lib e> /dev/null }
    }

    checkpoint "Stripped libs"

    # Include only the stdlib crates so rustc sees a sysroot
    let target_lib_dir = ($payload_dir | path join "lib" "rustlib" "x86_64-unknown-linux-gnu" "lib")
    mkdir $target_lib_dir

    let std_crates = [
        std        test          getopts      unicode_width
        core       alloc         panic_unwind panic_abort
        unwind     libc          compiler_builtins std_detect
        rustc_std_workspace_core rustc_std_workspace_alloc rustc_std_workspace_std
        miniz_oxide addr2line    object       gimli
        hashbrown  adler2        memchr       rustc_demangle
        rustc_literal_escaper cfg_if
        proc_macro
    ]

    let sysroot_lib_dir = ($sysroot | path join "lib" "rustlib" "x86_64-unknown-linux-gnu" "lib")

    for crate in $std_crates {
        let rlibs = (glob $"($sysroot_lib_dir)/lib($crate)-*.rlib")
        let rmetas = (glob $"($sysroot_lib_dir)/lib($crate)-*.rmeta")
        for f in ($rlibs | append $rmetas) {
            try { cp $f $target_lib_dir }
        }
    }

    # Strip .rlib archives (ar files)
    for rlib in (glob $"($target_lib_dir)/*.rlib") {
        let dir = (mktemp -d)
        cp $rlib ($dir | path join "archive.rlib")
        let orig_dir = $env.PWD
        cd $dir
        ar x archive.rlib
        
        let o_files = (glob "*.o")
        if ($o_files | length) > 0 {
            for o in $o_files {
                try { strip --strip-debug $o }
            }
        }
        let all_o = (glob "*.o")
        if ($all_o | length) > 0 {
            ar crs archive.rlib ...$all_o
            
        }
        cp archive.rlib $rlib
        cd $orig_dir
        rm -rf $dir
    }

    # Self‑contained linker components (if present)
    let linker_bin_dir = ($sysroot | path join "lib" "rustlib" "x86_64-unknown-linux-gnu" "bin")
    if ($linker_bin_dir | path exists) {
        let dest_bin = ($payload_dir | path join "lib" "rustlib" "x86_64-unknown-linux-gnu")
        mkdir $dest_bin
        cp -r $linker_bin_dir $dest_bin
    }

    # Rust standard library sources (enables `co2cargo miri run`)
    let library_src = ($sysroot | path join "lib" "rustlib" "src" "rust" "library")
    if ($library_src | path exists) {
        let dest_src = ($payload_dir | path join "lib" "rustlib" "src" "rust")
        mkdir $dest_src
        cp -r $library_src $dest_src
    }

    checkpoint "Finished payload dir"

    # 4. Create tarball (gzip by default, zstd if `--zstd` flag is given)
    let compress_flag = if $zstd { "--zstd" } else { "-z" }
    if $zstd {
        print "Using zstd compression"
    } else {
        print "Using gzip compression"
    }

    let tarball = (mktemp)
    tar -C $payload_dir -c $compress_flag -f $tarball .


    checkpoint "Created tarball"

    let hash = (open --raw $tarball | hash sha256)

    checkpoint "Evaluated hash of tarball"

    # 5. Create the self‑extracting script (the inner part stays Bash)
    let out_file = "target/co2-multicall.run"
    mkdir target

    let script_header = ([
        '#!/bin/bash'
        'set -e'
        ''
        ($"HASH=\"($hash)\"")
        'CACHE_DIR="$HOME/.cache/co2/$HASH"'
        ''
        'if [ ! -d "$CACHE_DIR" ]; then'
        '    # Find payload start - only when extracting'
        '    PAYLOAD_LINE=$(grep -a -n "^__PAYLOAD_BELOW__" "$0" | head -n 1 | cut -d: -f1)'
        '    PAYLOAD_START=$((PAYLOAD_LINE + 1))'
        '    mkdir -p "$CACHE_DIR"'
        ('    tail -n +$PAYLOAD_START "$0" | tar -x ' + $compress_flag + ' -C "$CACHE_DIR"')
        'fi'
        ''
        '# Multicall dispatch: use the name this script was called as'
        'ARG0="$0"'
        'export LD_LIBRARY_PATH="$CACHE_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"'
        'export CO2_RUN_SCRIPT="$(readlink -f "$0")"'
        '# Tell rustc where its sysroot is'
        'if [[ ! "$RUSTFLAGS" =~ "--sysroot" ]]; then'
        '    export RUSTFLAGS="--sysroot=$CACHE_DIR $RUSTFLAGS"'
        'fi'
        '# Provide stdlib sources for miri if not already set'
        'if [ -z "${MIRI_LIB_SRC}" ] && [ -d "$CACHE_DIR/lib/rustlib/src/rust/library" ]; then'
        '    export MIRI_LIB_SRC="$CACHE_DIR/lib/rustlib/src/rust/library"'
        'fi'
        ''
        '# Forward all arguments with preserved arg0'
        'exec -a "$ARG0" "$CACHE_DIR/bin/co2-multicall" "$@"'
    ] | str join (char newline))

    $script_header | save -f $out_file

    checkpoint "Created self extracting script"

    # Append the binary payload
    $"(char newline)__PAYLOAD_BELOW__(char newline)" | save --append $out_file
    open --raw $tarball | save --append $out_file

    chmod +x $out_file

    # Cleanup
    rm -rf $payload_dir $tarball

    checkpoint "Finished"

    print $"Created ($out_file)"
}
