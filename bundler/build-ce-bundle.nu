#!/usr/bin/env nu

source "prepare-payload.nu"

def main [--version: string] {
    let payload_dir = (prepare-payload --version $version)

    # 1. Create wrapper scripts in bin/ that source env.sh and forward to co2-multicall
    for applet in ["co2cc", "co2rustc", "co2miri", "co2fmt"] {
        let wrapper = ([
            '#!/bin/bash'
            'source "$(dirname "$(readlink -f "$0")")/../env.sh"'
            'exec -a "$0" "$(dirname "$(readlink -f "$0")")/co2-multicall" "$@"'
        ] | str join (char newline))

        $wrapper | save -f ($payload_dir | path join "bin" $applet)
        chmod +x ($payload_dir | path join "bin" $applet)
    }

    # 2. Create symlinks so co2cargo can find co2miri for miri sysroot setup
    ln -sf co2-multicall target/release/co2cargo
    ln -sf co2-multicall target/release/co2miri

    let sysroot = (rustc --print sysroot | str trim)
    let miri_sysroot = (with-env { LD_LIBRARY_PATH: $"($sysroot)/lib" } {
        ^target/release/co2cargo miri setup --print-sysroot
    } | str trim)
    cp -r $miri_sysroot ($payload_dir | path join "miri-sysroot")
    checkpoint "Copied miri sysroot"

    # 3. Create the CE tarball (zstd compressed)
    let out_file = "target/co2-ce.tar.zstd"
    mkdir target

    tar -C $payload_dir -c --zstd -f $out_file .

    checkpoint "Created CE tarball"

    rm -rf $payload_dir

    checkpoint "Finished"

    print $"Created ($out_file)"
}
