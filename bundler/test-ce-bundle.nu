#!/usr/bin/env nu

cd $env.FILE_PWD
cd ..

let ce_bundle = "target/co2-ce.tar.zstd"

print "Rebuilding CE bundle..."
nu ./bundler/build-ce-bundle.nu

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
    --ro-bind $ce_bundle /opt/co2-ce.tar.zstd
    --ro-bind ./bundler/ce-smoke-test.nu /opt/ce-smoke-test.nu
    --chdir /opt
    nu /opt/ce-smoke-test.nu
]
