#@ run-status: 0

let co2rustc = ($env.CO2_BIN_DIR | path join "co2rustc")
let test_dir = $env.CO2_TEST_DIR
let expected_dir = ($env.CO2_WORKSPACE_ROOT | path join "tests" "cli" "co2rustc" "emit_asm_rlib")

let foo_asm = ($test_dir | path join "foo.s")
let bar_asm = ($test_dir | path join "bar.s")

# compile foo.rs as rlib to assembly
let compile_foo = (do { ^$co2rustc foo.rs --crate-type=rlib -C opt-level=2 --emit=asm -o $foo_asm } | complete)
if $compile_foo.exit_code != 0 {
    print $"compiling foo.rs failed: ($compile_foo.stderr)"
    exit 1
}

# compile bar.rs as rlib to assembly
let compile_bar = (do { ^$co2rustc bar.rs --crate-type=rlib -C opt-level=2 --emit=asm -o $bar_asm } | complete)
if $compile_bar.exit_code != 0 {
    print $"compiling bar.rs failed: ($compile_bar.stderr)"
    exit 1
}

# read both assembly outputs, stripping .file lines, demangling, and normalizing crate names
let foo_raw = (open $foo_asm)
let bar_raw = (open $bar_asm)
let normalize = {|s|
    $s | ^c++filt | str replace --all --regex 'foo\[[^\]]*\]' 'crate_name' | str replace --all --regex 'bar\[[^\]]*\]' 'crate_name' | str replace --all --regex '_RNvCs[0-9a-zA-Z]+_3foo' '_RNvCsXXXX_3crate_name' | str replace --all --regex '_RNvCs[0-9a-zA-Z]+_3bar' '_RNvCsXXXX_3crate_name'
}
let foo_text = (do $normalize $foo_raw | lines | skip 1 | str join "\n")
let bar_text = (do $normalize $bar_raw | lines | skip 1 | str join "\n")

let snapshot = (open ($expected_dir | path join "func.snapshot"))

if $foo_text != $snapshot {
    print "FAIL: foo.s does not match snapshot!"
    print "--- foo.s ---"
    print $foo_text
    print "--- snapshot ---"
    print $snapshot
    exit 1
}

if $bar_text != $snapshot {
    print "FAIL: bar.s does not match snapshot!"
    print "--- bar.s ---"
    print $bar_text
    print "--- snapshot ---"
    print $snapshot
    exit 1
}

print "PASS"
exit 0
