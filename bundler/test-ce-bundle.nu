#!/usr/bin/env nu

def main [--version: string] {
    cd $env.FILE_PWD
    cd ..

    let ce_bundle = "target/co2-ce.tar.zstd"

    print $"Rebuilding CE bundle (version: ($version))..."
    nu ./bundler/build-ce-bundle.nu --version $version

    print "Testing CE bundle in bwrap container..."

    ^bwrap ...[
        --ro-bind /usr /usr
        --ro-bind /lib /lib
        --ro-bind /etc /etc
        --ro-bind /lib64 /lib64
        --ro-bind /bin /bin
        --dev /dev
        --proc /proc
        --tmpfs /tmp
        --tmpfs /opt
        --setenv CO2_EXPECTED_VERSION $version
        --ro-bind $ce_bundle /opt/co2-ce.tar.zstd
        --ro-bind ./bundler/ce-smoke-test.nu /opt/ce-smoke-test.nu
        --chdir /opt
        nu /opt/ce-smoke-test.nu
    ]
}
