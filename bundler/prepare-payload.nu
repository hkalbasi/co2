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

    checkpoint "Bundling musl headers"

    let musl_version = "1.2.6"
    let musl_sha256 = "d585fd3b613c66151fc3249e8ed44f77020cb5e6c1e635a616d3f9f82460512a"
    let musl_tarball = $"target/musl-($musl_version).tar.gz"

    if ($musl_tarball | path exists) {
        let cached_sha = (open --raw $musl_tarball | hash sha256)
        if $cached_sha != $musl_sha256 {
            print $"Cached musl tarball SHA mismatch, redownloading..."
            rm -f $musl_tarball
        } else {
            print "Using cached musl tarball"
        }
    }

    if not ($musl_tarball | path exists) {
        print $"Downloading musl ($musl_version)..."
        http get --raw $"https://musl.libc.org/releases/musl-($musl_version).tar.gz" | save -f $musl_tarball
        let actual_sha = (open --raw $musl_tarball | hash sha256)
        if $actual_sha != $musl_sha256 {
            error make { msg: $"musl tarball SHA mismatch after download: expected ($musl_sha256), got ($actual_sha)" }
        }
    }

    let include_dir = ($payload_dir | path join "include")
    mkdir $include_dir

    tar -xzf $musl_tarball -C $include_dir --strip-components=2 $"musl-($musl_version)/include/"
    rm -f ($include_dir | path join "alltypes.h.in")

    mkdir ($include_dir | path join "bits")
    # Extract generic bits first (as fallback), then x86_64-specific bits on top
    tar -xzf $musl_tarball -C ($include_dir | path join "bits") --strip-components=4 $"musl-($musl_version)/arch/generic/bits/"
    tar -xzf $musl_tarball -C ($include_dir | path join "bits") --strip-components=4 $"musl-($musl_version)/arch/x86_64/bits/"
    # Strip .in suffix from template headers (alltypes.h.in is processed separately below)
    for f in (glob ($include_dir | path join "bits" "*.in") | uniq) {
        if not ($f | str ends-with "alltypes.h.in") {
            mv $f ($f | str replace ".in" "")
        }
    }
    rm -f ($include_dir | path join "bits" "alltypes.h.in")

    let arch_types_file = (mktemp)
    let generic_types_file = (mktemp)
    let sed_script_file = (mktemp)
    let alltypes_out = ($include_dir | path join "bits" "alltypes.h")
    tar -xzf $musl_tarball --to-stdout $"musl-($musl_version)/arch/x86_64/bits/alltypes.h.in" | save -f $arch_types_file --raw
    tar -xzf $musl_tarball --to-stdout $"musl-($musl_version)/include/alltypes.h.in" | save -f $generic_types_file --raw
    tar -xzf $musl_tarball --to-stdout $"musl-($musl_version)/tools/mkalltypes.sed" | save -f $sed_script_file --raw
    ^bash -c $"cat ($arch_types_file) ($generic_types_file) | sed -f ($sed_script_file) > ($alltypes_out)"
    rm -f $arch_types_file $generic_types_file $sed_script_file

    let env_template = ($env.FILE_PWD | path join "env.sh")
    open $env_template | str replace "@CO2_VERSION@" $version | save -f ($payload_dir | path join "env.sh")

    checkpoint "Finished payload dir"

    return $payload_dir
}
