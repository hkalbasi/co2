#!/usr/bin/env nu

source "prepare-payload.nu"

def main [--version: string, --zstd] {

    let payload_dir = (prepare-payload --version $version)

    # 1. Create tarball (gzip by default, zstd if `--zstd` flag is given)
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

    # 2. Create the self‑extracting script (the inner part stays Bash)
    let out_file = "target/co2-multicall.run"
    mkdir target

    let script_header = ([
        '#!/bin/bash'
        'set -e'
        ''
        ($"HASH=\"($hash)\"")
        'CACHE_DIR="$HOME/.cache/co2/$HASH"'
        ''
        '# Self-extraction: extract tarball on first run'
        'if [ ! -d "$CACHE_DIR" ]; then'
        '    PAYLOAD_LINE=$(grep -a -n "^__PAYLOAD_BELOW__" "$0" | head -n 1 | cut -d: -f1)'
        '    PAYLOAD_START=$((PAYLOAD_LINE + 1))'
        '    mkdir -p "$CACHE_DIR"'
        ('    tail -n +$PAYLOAD_START "$0" | tar -x ' + $compress_flag + ' -C "$CACHE_DIR"')
        'fi'
        ''
        '# Set up environment via the extracted env.sh'
        'export CO2_CACHE_DIR="$CACHE_DIR"'
        'source "$CACHE_DIR/env.sh"'
        ''
        '# Multicall dispatch: use the name this script was called as'
        'ARG0="$0"'
        'export CO2_RUN_SCRIPT="$(readlink -f "$0")"'
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
