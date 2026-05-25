#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
mkdir ($test_dir | path join "co2cargo_incremental")
cd ($test_dir | path join "co2cargo_incremental")

let init = (do { ^co2cargo init test_project --bin } | complete)
if $init.exit_code != 0 {
    print $"co2cargo init failed: ($init.stderr)"
    exit 1
}

let project_dir = ($test_dir | path join "co2cargo_incremental" "test_project")
let main_co2 = ($project_dir | path join "src" "main.co2")
"int main(void) { return 0; }\n" | save -f $main_co2

cd $project_dir

let build1 = (do { ^co2cargo build } | complete)
if $build1.exit_code != 0 {
    print $"first co2cargo build failed: ($build1.stderr)"
    exit 2
}

((open $main_co2) + "\n// force rebuild\n") | save -f $main_co2

let build2 = (do { ^co2cargo build } | complete)
if $build2.exit_code != 0 {
    print $"second co2cargo build failed: ($build2.stderr)"
    exit 3
}

exit 0
