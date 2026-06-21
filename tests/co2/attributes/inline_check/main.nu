use ./snapshot-utils.nu *

let test_dir = $env.CO2_TEST_DIR
let rs_src = ($test_dir | path join "main.rs")
let rust_src = ($test_dir | path join "rust_inline.rs")
let bin = ($test_dir | path join "inline_attr_bin")
let rust_bin = ($test_dir | path join "rust_attr_bin")

# ---------- co2 ----------
let compile = (do { co2rustc $rs_src --crate-type=bin --edition=2024 -o $bin -C opt-level=2 -C remark=inline -C llvm-args=-pass-remarks-analysis=inline -C debuginfo=2 } | complete)
if $compile.exit_code != 0 {
    print $compile.stderr; exit 1
}
let run = (do { ^$bin } | complete)
if $run.exit_code != 0 {
    print $"co2 exit ($run.exit_code)"; exit 1
}

let our_funcs = [never_short always_short hint_short no_attr_short]
let actual_co2 = ($compile.stderr | lines | where {|line| $our_funcs | any {|f| ($line | str contains $f)} } | each {|line|
    $line | str replace --regex '^note: .+?main\.co2:' 'note: main.co2:' | str replace --regex 'inlined into [^ ]+' 'inlined into <caller>' | str replace --regex ' at callsite [^;]*;' ';'
} | sort | str join "\n")
assert-snapshot "co2 remark" $actual_co2 ($test_dir | path join "remark_snapshot.expected")

# ---------- Rust ----------
let rust_compile = (do { co2rustc $rust_src --crate-type=bin -o $rust_bin -C opt-level=2 -C remark=inline -C llvm-args=-pass-remarks-analysis=inline -C debuginfo=2 } | complete)
if $rust_compile.exit_code != 0 {
    print $rust_compile.stderr; exit 1
}
let rust_run = (do { ^$rust_bin } | complete)
if $rust_run.exit_code != 0 {
    print $"rust exit ($rust_run.exit_code)"; exit 1
}

let actual_rust = ($rust_compile.stderr | lines | where {|line| $our_funcs | any {|f| ($line | str contains $f)} } | each {|line|
    $line | str replace --regex '^note: .+?rust_inline\.rs:' 'note: rust_inline.rs:' | str replace --regex 'inlined into [^ ]+' 'inlined into <caller>' | str replace --regex ' at callsite [^;]*;' ';'
} | sort | str join "\n")
assert-snapshot "rust remark" $actual_rust ($test_dir | path join "rust_remark_snapshot.expected")

print "PASS"
exit 0
