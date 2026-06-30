#@ run-status: 0

use snapshot-utils.nu assert-snapshot

let install_sh = ($env.CO2_WORKSPACE_ROOT | path join "gh-pages" "install.sh")
let snap_dir = $env.CO2_TEST_SOURCE_DIR

# --- --help ---
let help = (do { ^bash $install_sh --help } | complete)
assert-snapshot "--help" $help.stdout ($snap_dir | path join "help.txt")
if $help.exit_code != 0 {
    print $"--help exit code: ($help.exit_code)"
    exit 1
}

# --- --version ---
let version = (do { ^bash $install_sh --version } | complete)
assert-snapshot "--version" $version.stdout ($snap_dir | path join "version.txt")
if $version.exit_code != 0 {
    print $"--version exit code: ($version.exit_code)"
    exit 1
}

# --- architecture detection (varies by host, inline check only) ---
let arch = (do { with-env { CO2_INIT_SH_PRINT: "arch" } { ^bash $install_sh } } | complete)
let arch_ok = (($arch.stdout | str contains "linux") or ($arch.stdout | str contains "darwin") or ($arch.stdout | str contains "freebsd"))
if $arch.exit_code != 0 or $arch_ok == false {
    print $"architecture detection failed: ($arch | to json -r)"
    exit 1
}

# --- dry-run default (trunk) ---
let dry = (do { ^bash $install_sh -y --install-dir /tmp/co2-test --dry-run } | complete)
assert-snapshot "dry-run default" $dry.stdout ($snap_dir | path join "dry_run_default.txt")
if $dry.exit_code != 0 {
    print $"dry-run exit code: ($dry.exit_code)"
    exit 1
}

# --- dry-run custom version ---
let ver = (do { with-env { CO2_VERSION: "2.3.4" } { ^bash $install_sh -y --install-dir /tmp/co2-test --dry-run } } | complete)
assert-snapshot "dry-run custom version" $ver.stdout ($snap_dir | path join "dry_run_custom_version.txt")
if $ver.exit_code != 0 {
    print $"CO2_VERSION exit code: ($ver.exit_code)"
    exit 1
}

# --- dry-run quiet mode ---
let quiet = (do { ^bash $install_sh -y -q --install-dir /tmp/co2-test --dry-run } | complete)
assert-snapshot "dry-run quiet" $quiet.stdout ($snap_dir | path join "dry_run_quiet.txt")
if $quiet.exit_code != 0 {
    print $"-q exit code: ($quiet.exit_code)"
    exit 1
}

# --- MOCK_RELEASES ---
let mock_releases = '[
  {"tag_name": "trunk"},
  {"tag_name": "5.0.0"},
  {"tag_name": "4.2.1"},
  {"tag_name": "3.0.0"},
  {"tag_name": "2.3.4"},
  {"tag_name": "2.0.0"},
  {"tag_name": "1.5.3"},
  {"tag_name": "1.2.0"},
  {"tag_name": "1.1.0"},
  {"tag_name": "1.0.0"},
  {"tag_name": "0.9.0"}
]'
let mock = (do { with-env { MOCK_RELEASES: $mock_releases } { ^bash $install_sh -y --install-dir /tmp/co2-test --dry-run } } | complete)
if $mock.exit_code != 0 {
    print $"MOCK_RELEASES caused failure: ($mock | to json -r)"
    exit 1
}

let input = $"2(char newline)(char newline)(char newline)3(char newline)"
let interactive = (
    do {
        with-env {
            MOCK_RELEASES: $mock_releases
            CO2_STDIN_FALLBACK: "1"
        } {
            $input | ^bash $install_sh --install-dir /tmp/co2-interactive-test --dry-run
        }
    } | complete
)
assert-snapshot "interactive customize then cancel" $interactive.stdout ($snap_dir | path join "interactive_customize_then_cancel.txt")
if $interactive.exit_code != 0 {
    print $"interactive test failed: ($interactive | to json -r)"
    exit 1
}

# --- unsupported architecture ---
let unsupported = (do { with-env { CO2_MOCK_ARCH: "powerpc64le-unknown-linux-gnu" } { ^bash $install_sh --dry-run --install-dir /tmp/co2-test } } | complete)
assert-snapshot "unsupported arch" $unsupported.stderr ($snap_dir | path join "unsupported_arch.txt")
if $unsupported.exit_code != 1 {
    print $"unsupported arch expected exit code 1, got ($unsupported.exit_code)"
    exit 1
}
