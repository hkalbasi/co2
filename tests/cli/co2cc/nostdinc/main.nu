#@ run-status: 0

let c_result = (do { co2cc -nostdinc inc_stdint.c } | complete)
if $c_result.exit_code == 0 {
    print $"`co2cc -nostdinc inc_stdint.c` unexpectedly succeeded: ($c_result)"
    exit 1
}
