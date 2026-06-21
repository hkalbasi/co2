# Test: co2cargo miri run detects null-pointer UB in a co2 project.
#
# Skips gracefully when cargo-miri is not installed on the system.
# No extra environment variables are required: co2cargo auto-detects
# cargo-miri via `rustup which`, mirroring the plain `cargo miri run`
# user experience.

use ./snapshot-utils.nu *

let bin_dir = $env.CO2_BIN_DIR
let test_dir = $env.CO2_TEST_DIR

# Skip if cargo-miri is not available.  co2cargo will also handle this
# gracefully, but we skip early here to give a cleaner message.
let miri_check = (do { ^rustup which cargo-miri } | complete)
if $miri_check.exit_code != 0 {
    print "skip: cargo-miri not available (install miri: rustup component add miri)"
    exit 0
}

# ── Create a minimal co2 project ──────────────────────────────────────────────
let project = ($test_dir | path join "null_deref_project")
mkdir $project

"[package]
name = \"null_deref_project\"
version = \"0.1.0\"
edition = \"2024\"
" | save ($project | path join "Cargo.toml")

mkdir ($project | path join "src")

# Rust shim that declares the co2 language
"#![language(co2)]
" | save ($project | path join "src" "main.rs")

# co2 source with null-pointer dereference (UB)
"fn main() {
    i32* ptr = std::ptr::null_mut::<i32>();
    *ptr = 1;
}
" | save ($project | path join "src" "main.co2")

# ── Run miri ──────────────────────────────────────────────────────────────────
cd $project

# co2cargo auto-detects cargo-miri and co2miri; no extra env vars needed.
# Any env vars already in the shell (e.g. MIRI_LIB_SRC for dev setups with
# mismatched toolchain sources) are inherited naturally.
let result = (do { ^co2cargo miri run } | complete)

# miri must exit non-zero when UB is detected
if $result.exit_code == 0 {
    print "FAIL: miri exited 0 — expected non-zero exit for UB"
    print $"stdout: ($result.stdout)"
    print $"stderr: ($result.stderr)"
    exit 1
}

# ── Normalize and compare stderr ──────────────────────────────────────────────
# Miri emits an absolute path in the location line; normalise it to the
# project-relative form ("src/main.co2") so the comparison is reproducible.
# Also strip cargo build noise (sysroot prep, "Compiling ...", "Finished ...",
# "Running ..." lines) so only the miri diagnostic remains.
let stderr_lines = ($result.stderr | lines)
let first_error_idx = ($stderr_lines | enumerate | where { |it| $it.item | str starts-with "error:" } | get -o 0.index)
let stderr_diagnostic = if ($first_error_idx | is-not-empty) {
    $stderr_lines | skip $first_error_idx | str join "\n"
} else {
    $result.stderr
}
let stderr_normalized = (
    $stderr_diagnostic
    | str replace --regex ' --> .+src/main\.co2:' ' --> src/main.co2:'
)

let expected_path = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2cargo" "miri_null_deref" "stderr.expected")
assert-snapshot "miri stderr" $stderr_normalized $expected_path

print "co2cargo miri null-deref UB detection test passed"
exit 0
