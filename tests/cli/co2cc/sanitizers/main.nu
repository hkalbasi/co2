#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
let ok_src = ($test_dir | path join "ok.c")
let boom_src = ($test_dir | path join "overflow.c")
let ok_bin = ($test_dir | path join "ok_asan")
let boom_bin = ($test_dir | path join "boom_asan")
let boom_plain = ($test_dir | path join "boom_plain")
let boom_obj = ($test_dir | path join "boom_asan.o")
let boom_split = ($test_dir | path join "boom_split")

# Clean program with ASan must build and run cleanly.
let ok_c = (do { co2cc -fsanitize=address $ok_src -o $ok_bin } | complete)
if $ok_c.exit_code != 0 {
    print $"co2cc -fsanitize=address failed: ($ok_c.stderr)"
    exit 1
}
let ok_run = (do { ^$ok_bin } | complete)
if $ok_run.exit_code != 0 {
    print $"ASan binary exited with ($ok_run.exit_code): ($ok_run.stderr)"
    exit 1
}

# Heap-buffer-overflow must abort under ASan.
let boom_c = (do { co2cc -fsanitize=address $boom_src -o $boom_bin } | complete)
if $boom_c.exit_code != 0 {
    print $"co2cc -fsanitize=address failed: ($boom_c.stderr)"
    exit 2
}
let boom_run = (do { ^$boom_bin } | complete)
if $boom_run.exit_code == 0 {
    print "ASan binary did not report the heap-buffer-overflow"
    exit 3
}
let boom_report = ($boom_run.stderr + "\n" + $boom_run.stdout)
if ($boom_report | str contains "AddressSanitizer") == false {
    print $"expected AddressSanitizer report, got: ($boom_report)"
    exit 4
}

# Control: without the flag the same program runs cleanly.
let plain_c = (do { co2cc $boom_src -o $boom_plain } | complete)
if $plain_c.exit_code != 0 {
    print $"co2cc without sanitizer failed: ($plain_c.stderr)"
    exit 5
}
let plain_run = (do { ^$boom_plain } | complete)
if $plain_run.exit_code != 0 {
    print $"plain binary exited with ($plain_run.exit_code): ($plain_run.stderr)"
    exit 6
}

# Split -c/object + link path must also instrument and link the runtime.
let obj_c = (do { co2cc -c -fsanitize=address $boom_src -o $boom_obj } | complete)
if $obj_c.exit_code != 0 {
    print $"co2cc -c -fsanitize=address failed: ($obj_c.stderr)"
    exit 7
}
let split_c = (do { co2cc -fsanitize=address $boom_obj -o $boom_split } | complete)
if $split_c.exit_code != 0 {
    print $"co2cc link with -fsanitize=address failed: ($split_c.stderr)"
    exit 8
}
let split_run = (do { ^$boom_split } | complete)
if $split_run.exit_code == 0 {
    print "split object/link ASan binary did not report the heap-buffer-overflow"
    exit 9
}
let split_report = ($split_run.stderr + "\n" + $split_run.stdout)
if ($split_report | str contains "AddressSanitizer") == false {
    print $"expected AddressSanitizer report, got: ($split_report)"
    exit 10
}

let macros_src = ($test_dir | path join "sanitizer_macros.c")

# __SANITIZE_ADDRESS__ must be absent without the flag.
let macros_noflag = (do { co2cc $macros_src -o ($test_dir | path join "macros_noflag") } | complete)
if $macros_noflag.exit_code == 0 {
    print "co2cc without -fsanitize unexpectedly defined __SANITIZE_ADDRESS__"
    exit 11
}

# __SANITIZE_ADDRESS__ must be defined with -fsanitize=address.
let macros_flag = (do { co2cc -fsanitize=address $macros_src -o ($test_dir | path join "macros_asan") } | complete)
if $macros_flag.exit_code != 0 {
    print $"co2cc -fsanitize=address macro check failed: ($macros_flag.stderr)"
    exit 12
}
let macros_run = (do { ^($test_dir | path join "macros_asan") } | complete)
if $macros_run.exit_code != 0 {
    print $"macro check binary exited with ($macros_run.exit_code)"
    exit 13
}

# Explicit -U must still win over the predefined sanitizer macro.
let macros_undef = (do { co2cc -fsanitize=address -U __SANITIZE_ADDRESS__ $macros_src -o ($test_dir | path join "macros_undef") } | complete)
if $macros_undef.exit_code == 0 {
    print "-U __SANITIZE_ADDRESS__ did not override the predefined macro"
    exit 14
}

# Unknown sanitizers must be rejected with the valid values listed.
let bogus = (do { co2cc -fsanitize=bogus $ok_src -o ($test_dir | path join "bogus") } | complete)
if $bogus.exit_code == 0 {
    print "co2cc -fsanitize=bogus unexpectedly succeeded"
    exit 15
}
if ($bogus.stderr | str contains "'-fsanitize=bogus'") == false {
    print $"expected offending flag in error, got: ($bogus.stderr)"
    exit 16
}
if ($bogus.stderr | str contains "valid values") == false {
    print $"expected valid-values hint, got: ($bogus.stderr)"
    exit 17
}

# `undefined` has no rustc equivalent: reject, don't silently drop.
let undef_san = (do { co2cc -fsanitize=address,undefined $ok_src -o ($test_dir | path join "undef_san") } | complete)
if $undef_san.exit_code == 0 {
    print "co2cc -fsanitize=address,undefined unexpectedly succeeded"
    exit 18
}

# Empty and negated-bogus lists must be rejected too.
let empty = (do { co2cc "-fsanitize=" $ok_src -o ($test_dir | path join "empty_san") } | complete)
if $empty.exit_code == 0 {
    print "co2cc -fsanitize= unexpectedly succeeded"
    exit 19
}
let neg_bogus = (do { co2cc -fno-sanitize=bogus $ok_src -o ($test_dir | path join "neg_bogus") } | complete)
if $neg_bogus.exit_code == 0 {
    print "co2cc -fno-sanitize=bogus unexpectedly succeeded"
    exit 20
}

# Every other sanitizer that works on this host must build and run a clean
# program cleanly. (`address` is covered above; `cfi` needs `-flto` and is
# covered in the CFI section below.)
for san in [kcfi leak memory realtime safestack thread] {
    let bin = ($test_dir | path join $"clean_($san)")
    let c = (do { co2cc $"-fsanitize=($san)" $ok_src -o $bin } | complete)
    if $c.exit_code != 0 {
        print $"co2cc -fsanitize=($san) failed: ($c.stderr)"
        exit 21
    }
    let run = (do { ^$bin } | complete)
    if $run.exit_code != 0 {
        print $"clean -fsanitize=($san) binary exited with ($run.exit_code): ($run.stderr)"
        exit 22
    }
}

# `dataflow` instruments at -c (used for IR inspection, like clang's own
# dataflow tests) but cannot link: neither co2cc nor native rustc ships a
# dataflow `main` trampoline, so the final link has no `main` symbol.
let df_obj = (do { co2cc -c -fsanitize=dataflow $ok_src -o ($test_dir | path join "df.o") } | complete)
if $df_obj.exit_code != 0 {
    print $"co2cc -c -fsanitize=dataflow failed: ($df_obj.stderr)"
    exit 23
}
let df_bin = (do { co2cc -fsanitize=dataflow $ok_src -o ($test_dir | path join "df_bin") } | complete)
if $df_bin.exit_code == 0 {
    print "co2cc -fsanitize=dataflow link unexpectedly succeeded"
    exit 24
}

# Sanitizers with no host-target support must fail with rustc's clear
# diagnostic, not a silent miscompile.
for san in [hwaddress kernel-address kernel-hwaddress memtag shadow-call-stack] {
    let c = (do { co2cc $"-fsanitize=($san)" $ok_src -o ($test_dir | path join $"unsup_($san)") } | complete)
    if $c.exit_code == 0 {
        print $"co2cc -fsanitize=($san) unexpectedly succeeded on this target"
        exit 25
    }
    if ($c.stderr | str contains "not supported for this target") == false {
        print $"expected target-unsupported diagnostic for ($san), got: ($c.stderr)"
        exit 26
    }
}

# Data race must be reported under ThreadSanitizer.
let race_bin = ($test_dir | path join "race_tsan")
let race_c = (do { co2cc -fsanitize=thread ($test_dir | path join "race.c") -o $race_bin } | complete)
if $race_c.exit_code != 0 {
    print $"co2cc -fsanitize=thread race build failed: ($race_c.stderr)"
    exit 27
}
let race_run = (do { ^$race_bin } | complete)
if $race_run.exit_code == 0 {
    print "ThreadSanitizer binary did not report the data race"
    exit 28
}
if ((($race_run.stderr + "\n" + $race_run.stdout) | str contains "ThreadSanitizer") == false) {
    print $"expected ThreadSanitizer report, got: ($race_run.stderr)"
    exit 28
}

# Uninitialized read must be reported under MemorySanitizer.
let uninit_bin = ($test_dir | path join "uninit_msan")
let uninit_c = (do { co2cc -fsanitize=memory ($test_dir | path join "uninit.c") -o $uninit_bin } | complete)
if $uninit_c.exit_code != 0 {
    print $"co2cc -fsanitize=memory build failed: ($uninit_c.stderr)"
    exit 29
}
let uninit_run = (do { ^$uninit_bin } | complete)
if $uninit_run.exit_code == 0 {
    print "MemorySanitizer binary did not report the uninitialized read"
    exit 30
}
if ((($uninit_run.stderr + "\n" + $uninit_run.stdout) | str contains "MemorySanitizer") == false) {
    print $"expected MemorySanitizer report, got: ($uninit_run.stderr)"
    exit 30
}

# Leaked allocation must be reported under LeakSanitizer.
let leak_bin = ($test_dir | path join "leak_lsan")
let leak_c = (do { co2cc -fsanitize=leak ($test_dir | path join "leak.c") -o $leak_bin } | complete)
if $leak_c.exit_code != 0 {
    print $"co2cc -fsanitize=leak build failed: ($leak_c.stderr)"
    exit 31
}
let leak_run = (do { ^$leak_bin } | complete)
if $leak_run.exit_code == 0 {
    print "LeakSanitizer binary did not report the leak"
    exit 32
}
if ((($leak_run.stderr + "\n" + $leak_run.stdout) | str contains "LeakSanitizer") == false) {
    print $"expected LeakSanitizer report, got: ($leak_run.stderr)"
    exit 32
}

# Type-confused indirect call must trap under CFI.
let cfi_src = ($test_dir | path join "cfi.c")
let cfi_bin = ($test_dir | path join "cfi_bin")

# Like clang, -fsanitize=cfi without -flto must fail fast.
let cfi_nolto = (do { co2cc -fsanitize=cfi $cfi_src -o ($test_dir | path join "cfi_nolto") } | complete)
if $cfi_nolto.exit_code == 0 {
    print "co2cc -fsanitize=cfi without -flto unexpectedly succeeded"
    exit 40
}
if ($cfi_nolto.stderr | str contains "only allowed with '-flto'") == false {
    print $"expected -flto requirement diagnostic, got: ($cfi_nolto.stderr)"
    exit 41
}

let cfi_c = (do { co2cc -flto -fsanitize=cfi $cfi_src -o $cfi_bin } | complete)
if $cfi_c.exit_code != 0 {
    print $"co2cc -flto -fsanitize=cfi failed: ($cfi_c.stderr)"
    exit 33
}
# The trap is SIGILL, which nushell reports as `core_dumped` instead of
# capturing, so run through a wrapper that prints the outcome as a marker.
let cfi_run = (do { bash -c 'ulimit -c 0; "$1"; echo "ec=$?"' _ $cfi_bin } | complete)
if $cfi_run.exit_code != 0 {
    print $"CFI wrapper failed: ($cfi_run.stderr)"
    exit 34
}
if ($cfi_run.stdout | str contains "ec=0") {
    print "CFI binary did not trap the type-confused indirect call"
    exit 35
}
let cfi_plain_c = (do { co2cc $cfi_src -o ($test_dir | path join "cfi_plain") } | complete)
if $cfi_plain_c.exit_code != 0 {
    print $"co2cc without sanitizer failed: ($cfi_plain_c.stderr)"
    exit 36
}
let cfi_plain_run = (do { ^($test_dir | path join "cfi_plain") } | complete)
if $cfi_plain_run.exit_code != 0 {
    print $"plain CFI-program binary exited with ($cfi_plain_run.exit_code): ($cfi_plain_run.stderr)"
    exit 36
}

# __SANITIZE_THREAD__ / __SANITIZE_LEAK__ mirror the address-macro checks.
for spec in [{san: "thread", mac: "__SANITIZE_THREAD__"} {san: "leak", mac: "__SANITIZE_LEAK__"}] {
    let src = ($test_dir | path join $"($spec.san)_macros.c")
    let noflag = (do { co2cc $src -o ($test_dir | path join $"($spec.san)_macros_noflag") } | complete)
    if $noflag.exit_code == 0 {
        print $"co2cc without -fsanitize unexpectedly defined ($spec.mac)"
        exit 37
    }
    let flagged = (do { co2cc $"-fsanitize=($spec.san)" $src -o ($test_dir | path join $"($spec.san)_macros") } | complete)
    if $flagged.exit_code != 0 {
        print $"co2cc -fsanitize=($spec.san) macro check failed: ($flagged.stderr)"
        exit 38
    }
    let flagged_run = (do { ^($test_dir | path join $"($spec.san)_macros") } | complete)
    if $flagged_run.exit_code != 0 {
        print $"($spec.mac) check binary exited with ($flagged_run.exit_code)"
        exit 39
    }
}

# -flto alone must build and run a clean program cleanly.
let lto_bin = ($test_dir | path join "clean_lto")
let lto_c = (do { co2cc -flto $ok_src -o $lto_bin } | complete)
if $lto_c.exit_code != 0 {
    print $"co2cc -flto failed: ($lto_c.stderr)"
    exit 42
}
let lto_run = (do { ^$lto_bin } | complete)
if $lto_run.exit_code != 0 {
    print $"clean -flto binary exited with ($lto_run.exit_code): ($lto_run.stderr)"
    exit 43
}

# Only full LTO is supported: ThinLTO cannot work with prebuilt objects.
let thin = (do { co2cc -flto=thin $ok_src -o ($test_dir | path join "thin_bin") } | complete)
if $thin.exit_code == 0 {
    print "co2cc -flto=thin unexpectedly succeeded"
    exit 44
}
if ($thin.stderr | str contains "only full LTO is supported") == false {
    print $"expected full-LTO-only diagnostic, got: ($thin.stderr)"
    exit 45
}

exit 0
