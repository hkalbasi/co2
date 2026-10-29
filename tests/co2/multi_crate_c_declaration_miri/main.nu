#@ run-status: 0

let workspace = ($env.CO2_TEST_DIR | path join "workspace")

cd $workspace

let miri_run = (do { ^co2cargo miri run -p app } | complete)
if $miri_run.exit_code != 0 {
    print $"co2cargo miri run failed with exit status ($miri_run.exit_code): ($miri_run.stdout) ($miri_run.stderr)"
    exit 1
}

print "co2cargo miri C declaration cross-crate call OK"
