#@ run-status: 0

let co2cc = ($env.CO2_BIN_DIR | path join "co2cc")
let base_env = { CARGO_MANIFEST_DIR: $env.CO2_TEST_DIR }

"#warning \"heads up\"
int main(void) {
    return 0;
}
" | save -f warn.c

let warning = (do { with-env $base_env { ^$co2cc warn.c -o warn } } | complete)
if $warning.exit_code != 0 {
    print $"co2cc warning compile failed: ($warning | to json -r)"
    exit 1
}
if ($warning.stderr | str contains "\"$message_type\"") {
    print $"expected human warning diagnostics, got JSON: ($warning.stderr)"
    exit 1
}
if ($warning.stderr | str contains "#warning \"heads up\"") == false or ($warning.stderr | str contains "warn.c") == false {
    print $"co2cc human warning output did not mention the warning source: ($warning.stderr)"
    exit 1
}

let warn_run = (do { ./warn } | complete)
if $warn_run.exit_code != 0 {
    print $"warning case executable failed to run: ($warn_run | to json -r)"
    exit 1
}

"int main(void) {
    return missing;
}
" | save -f unresolved.c

let unresolved = (do { with-env $base_env { ^$co2cc unresolved.c -o unresolved } } | complete)
if $unresolved.exit_code != 5 {
    print $"co2cc unresolved-name diagnostics used the wrong exit code: ($unresolved | to json -r)"
    exit 1
}
if ($unresolved.stderr | str contains "\"$message_type\"") {
    print $"expected human unresolved-name diagnostics, got JSON: ($unresolved.stderr)"
    exit 1
}
if ($unresolved.stderr | str contains "Unresolved name missing") == false or ($unresolved.stderr | str contains "unresolved.c") == false {
    print $"co2cc human unresolved-name diagnostic is missing expected text: ($unresolved.stderr)"
    exit 1
}

"@
" | save -f bad_lexer.h
"#include \"bad_lexer.h\"
int main(void) {
    return 0;
}
" | save -f lexer_main.c

let lexer = (do { with-env $base_env { ^$co2cc lexer_main.c -o lexer } } | complete)
if $lexer.exit_code != 5 {
    print $"co2cc lexer error path used the wrong exit code: ($lexer | to json -r)"
    exit 1
}
if ($lexer.stderr | str contains "\"$message_type\"") {
    print $"expected human lexer diagnostics, got JSON: ($lexer.stderr)"
    exit 1
}
if ($lexer.stderr | str contains "bad_lexer.h") == false {
    print $"co2cc lexer diagnostic did not point at the included header: ($lexer.stderr)"
    exit 1
}
