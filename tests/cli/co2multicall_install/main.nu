#@ run-status: 0

let install_dir = ($env.PWD | path join "fresh-install" "bin")
let multicall = ($env.CO2_BIN_DIR | path join "co2-multicall")

let usage = (do { ^$multicall } | complete)
if $usage.exit_code != 2 or ($usage.stderr | str contains "usage: co2-multicall install") == false {
    print $"co2-multicall usage path failed: ($usage | to json -r)"
    exit 1
}

let install = (do { with-env { CO2_RUN_SCRIPT: $multicall } { ^$multicall install $install_dir } } | complete)
if $install.exit_code != 0 or ($install.stdout | str contains "Successfully installed to") == false {
    print $"co2-multicall install failed: ($install | to json -r)"
    exit 1
}

for name in ["co2-multicall" "co2cc" "co2cargo" "co2miri" "co2rustc" "co2rustdoc"] {
    let path = ($install_dir | path join $name)
    if ($path | path exists) == false {
        print $"missing installed applet: ($path)"
        exit 1
    }
}

let cargo_help = (do { ^($install_dir | path join "co2cargo") --help } | complete)
if $cargo_help.exit_code != 0 or ($cargo_help.stdout | str contains "CO2 Specific Commands") == false {
    print $"installed co2cargo symlink is broken: ($cargo_help | to json -r)"
    exit 1
}

let reinstall = (do { with-env { CO2_RUN_SCRIPT: $multicall } { ^$multicall install $install_dir } } | complete)
if $reinstall.exit_code != 0 or ($reinstall.stdout | str contains "Successfully installed to") == false {
    print $"co2-multicall reinstall path failed: ($reinstall | to json -r)"
    exit 1
}

let replace_dir = ($env.PWD | path join "install-replace-dir")
mkdir $replace_dir
mkdir ($replace_dir | path join "co2cc")
let replace_fail = (do { with-env { CO2_RUN_SCRIPT: $multicall } { ^$multicall install $replace_dir } } | complete)
if $replace_fail.exit_code != 1 or ($replace_fail.stderr | str contains "refusing to replace directory") == false {
    print $"replace-directory error path failed: ($replace_fail | to json -r)"
    exit 1
}

let unknown = (do { with-env { CO2_APPLET_OVERRIDE: "totally-unknown-applet" } { ^$multicall ignored } } | complete)
if $unknown.exit_code != 2 or ($unknown.stderr | str contains "unknown applet") == false {
    print $"unknown-applet path failed: ($unknown | to json -r)"
    exit 1
}

let unknown_install_dir = ($env.PWD | path join "unknown-install" "bin")
let unknown_install = (
    do {
        with-env { CO2_APPLET_OVERRIDE: "totally-unknown-applet" CO2_RUN_SCRIPT: $multicall } {
            ^$multicall install $unknown_install_dir
        }
    } | complete
)
if $unknown_install.exit_code != 0 or ($unknown_install.stdout | str contains "Successfully installed to") == false {
    print $"unknown-applet install dispatch path failed: ($unknown_install | to json -r)"
    exit 1
}
