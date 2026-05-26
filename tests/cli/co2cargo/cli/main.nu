#@ run-status: 0

let fake_bin = ($env.PWD | path join "fake-bin")
mkdir $fake_bin
let tmp_root = ((mktemp -d) | str trim)
let co2cargo_bin = ($env.CO2_BIN_DIR | path join "co2cargo")

let cargo_miri = ($fake_bin | path join "cargo-miri")
[
    "#!/bin/sh"
    "printf '%s\\n' \"$@\" > cargo-miri.args"
    "printf 'MIRI=%s\\nRUSTC=%s\\nCARGO_INCREMENTAL=%s\\nRUSTFLAGS=%s\\n' \"${MIRI-}\" \"${RUSTC-}\" \"${CARGO_INCREMENTAL-}\" \"${RUSTFLAGS-}\" > cargo-miri.env"
] | str join "\n" | save -f $cargo_miri
chmod +x $cargo_miri

let help = (do { co2cargo --help } | complete)
if $help.exit_code != 0 or ($help.stdout | str contains "CO2 Specific Commands") == false {
    print $"co2cargo --help failed: ($help.stderr)"
    exit 1
}

let no_args = (do { co2cargo } | complete)
if $no_args.exit_code != 1 or ($no_args.stderr | str contains "usage: co2cargo") == false {
    print $"co2cargo without args behaved unexpectedly: ($no_args | to json -r)"
    exit 1
}

let current_init = ($tmp_root | path join "current-init")
mkdir $current_init
cd $current_init
let init_in_place = (do { co2cargo init } | complete)
if $init_in_place.exit_code != 0 {
    print $"co2cargo init without a path failed: ($init_in_place | to json -r)"
    exit 1
}
if (open ($current_init | path join "src/main.rs") | str trim) != "#![language(co2)]" {
    print "co2cargo init in place did not rewrite src/main.rs"
    exit 1
}
if (open ($current_init | path join "src/main.co2") | str trim) != "fn main() {}" {
    print "co2cargo init in place did not create src/main.co2"
    exit 1
}

let explicit_init = ($tmp_root | path join "explicit-init")
let init = (do { co2cargo init $explicit_init } | complete)
if $init.exit_code != 0 {
    print $"co2cargo init with an explicit path failed: ($init.stderr)"
    exit 1
}

if (open ($explicit_init | path join "src/main.rs") | str trim) != "#![language(co2)]" {
    print "co2cargo init did not rewrite src/main.rs"
    exit 1
}
if (open ($explicit_init | path join "src/main.co2") | str trim) != "fn main() {}" {
    print "co2cargo init did not create src/main.co2"
    exit 1
}

let init_fail_dir = ($tmp_root | path join "init-fail")
mkdir $init_fail_dir
"not a cargo project" | save -f ($init_fail_dir | path join "Cargo.toml")
let init_fail = (do { co2cargo init $init_fail_dir } | complete)
if $init_fail.exit_code != 1 or ($init_fail.stderr | str contains "co2cargo init failed: cargo init failed:") == false {
    print $"co2cargo init failure path missing: ($init_fail | to json -r)"
    exit 1
}

let miri = (do {
    with-env {
        PATH: ($"($fake_bin):($env.PATH)")
        RUSTFLAGS: "--cfg demo --sysroot /tmp/fake-sysroot -C opt-level=1"
    } { ^$co2cargo_bin miri run --quiet }
} | complete)
if $miri.exit_code != 0 {
    print $"co2cargo miri wrapper failed: ($miri.stderr)"
    exit 1
}

let args = (open cargo-miri.args)
if ($args | str contains "miri\nrun\n--quiet") == false {
    print $"unexpected cargo-miri args: ($args)"
    exit 1
}

let env_dump = (open cargo-miri.env)
if ($env_dump | str contains "RUSTC=co2rustc") == false {
    print $"co2cargo miri did not force RUSTC=co2rustc: ($env_dump)"
    exit 1
}
if ($env_dump | str contains "CARGO_INCREMENTAL=0") == false {
    print $"co2cargo miri did not force CARGO_INCREMENTAL=0: ($env_dump)"
    exit 1
}
if ($env_dump | str contains "--sysroot") == true {
    print $"co2cargo miri did not strip --sysroot from RUSTFLAGS: ($env_dump)"
    exit 1
}
if ($env_dump | str contains "--cfg demo -C opt-level=1") == false {
    print $"co2cargo miri dropped unrelated RUSTFLAGS: ($env_dump)"
    exit 1
}

let cargo = ($fake_bin | path join "cargo")
[
    "#!/bin/sh"
    "printf '%s\\n' \"$@\" > cargo.args"
    "printf 'RUSTDOC=%s\\nRUSTC=%s\\nCARGO_INCREMENTAL=%s\\n' \"${RUSTDOC-}\" \"${RUSTC-}\" \"${CARGO_INCREMENTAL-}\" > cargo.env"
] | str join "\n" | save -f $cargo
chmod +x $cargo

let doc = (do {
    with-env {
        PATH: ($"($fake_bin):($env.PATH)")
    } { ^$co2cargo_bin doc --no-deps }
} | complete)
if $doc.exit_code != 0 {
    print $"co2cargo doc wrapper failed: ($doc.stderr)"
    exit 1
}

let cargo_args = (open cargo.args)
if ($cargo_args | str contains "doc\n--no-deps") == false {
    print $"unexpected cargo doc args: ($cargo_args)"
    exit 1
}

let cargo_env = (open cargo.env)
if ($cargo_env | str contains "RUSTC=co2rustc") == false {
    print $"co2cargo doc did not force RUSTC=co2rustc: ($cargo_env)"
    exit 1
}
if ($cargo_env | str contains "CARGO_INCREMENTAL=0") == false {
    print $"co2cargo doc did not force CARGO_INCREMENTAL=0: ($cargo_env)"
    exit 1
}
if ($cargo_env | str contains "co2rustdoc") == false {
    print $"co2cargo doc did not force RUSTDOC=co2rustdoc: ($cargo_env)"
    exit 1
}
