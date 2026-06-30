#!/usr/bin/env nu

# Helper to print progress with timing.
def --env checkpoint [message: string] {
    let now = (date now | into int)
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

# Build co2-multicall and prepare the payload directory with all libraries
# and rust stdlib crates. Returns the path to the prepared payload directory.
def --env prepare-payload [--version: string] {
    checkpoint "Starting"

    cargo build -p co2-multicall --release

    checkpoint "Build successfully"

    let payload_dir = (mktemp -d)
    mkdir ($payload_dir | path join "bin")
    mkdir ($payload_dir | path join "lib")

    cp target/release/co2-multicall ($payload_dir | path join "bin" "co2-multicall")

    checkpoint "Prepared payload dir"

    let sysroot = (rustc --print sysroot | str trim)

    let driver_sos = (glob $"($sysroot)/lib/librustc_driver-*.so")
    for f in $driver_sos {
        cp $f ($payload_dir | path join "lib")
    }

    let llvm_sos = (glob $"($sysroot)/lib/libLLVM*.so*")
    for f in $llvm_sos {
        cp $f ($payload_dir | path join "lib")
    }

    checkpoint "Collected libs"

    let so_files = (glob $"($payload_dir)/lib/*.so*")
    for lib in $so_files {
        try { strip --strip-debug $lib e> /dev/null }
    }

    checkpoint "Stripped libs"

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
        proc_macro profiler_builtins
    ]

    let sysroot_lib_dir = ($sysroot | path join "lib" "rustlib" "x86_64-unknown-linux-gnu" "lib")

    for crate in $std_crates {
        let rlibs = (glob $"($sysroot_lib_dir)/lib($crate)-*.rlib")
        let rmetas = (glob $"($sysroot_lib_dir)/lib($crate)-*.rmeta")
        for f in ($rlibs | append $rmetas) {
            try { cp $f $target_lib_dir }
        }
    }

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

    let linker_bin_dir = ($sysroot | path join "lib" "rustlib" "x86_64-unknown-linux-gnu" "bin")
    if ($linker_bin_dir | path exists) {
        let dest_bin = ($payload_dir | path join "lib" "rustlib" "x86_64-unknown-linux-gnu")
        mkdir $dest_bin
        cp -r $linker_bin_dir $dest_bin
    }

    let library_src = ($sysroot | path join "lib" "rustlib" "src" "rust" "library")
    if ($library_src | path exists) {
        let dest_src = ($payload_dir | path join "lib" "rustlib" "src" "rust")
        mkdir $dest_src
        cp -r $library_src $dest_src
    }

    let env_template = ($env.FILE_PWD | path join "env.sh")
    open $env_template | str replace "@CO2_VERSION@" $version | save -f ($payload_dir | path join "env.sh")

    checkpoint "Finished payload dir"

    return $payload_dir
}
