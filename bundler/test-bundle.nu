#!/usr/bin/env nu

def main [--version: string] {
    cd $env.FILE_PWD
    cd ..

    let bundle = "target/co2-multicall.run"
    let stable_cargo = (rustup which --toolchain stable cargo | str trim)

    print $"Rebuilding bundle ..."
    nu ./bundler/build-bundle.nu --version $version --zstd

    print "Testing bundle in bwrap container..."

    ^bwrap ...[
        --ro-bind /usr /usr
        --ro-bind /lib /lib
        --ro-bind /etc /etc
        --ro-bind /lib64 /lib64
        --ro-bind /bin /bin
        --dev /dev
        --proc /proc
        --tmpfs /tmp
        --tmpfs /home
        --setenv HOME /home/testuser
        --setenv CO2_EXPECTED_VERSION $version
        --dir /home/testuser
        --bind $bundle /test/co2-multicall.run
        --ro-bind $stable_cargo /test/stable-cargo
        --ro-bind ./bundler/bundle-smoke-test.nu /test/bundle-smoke-test.nu
        --chdir /test
        nu /test/bundle-smoke-test.nu
    ]
}
