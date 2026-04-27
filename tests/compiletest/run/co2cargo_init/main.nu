# Test co2cargo init for binary crate
let test_dir = $env.CO2_TEST_DIR
mkdir ($test_dir | path join "co2cargo_init_test")
cd ($test_dir | path join "co2cargo_init_test")

# Run co2cargo via co2-multicall symlink (co2-multicall is in PATH via CO2_BIN_DIR)
let status = (do { ^co2cargo init test_project --bin } | complete)
if $status.exit_code != 0 {
    print $"co2cargo init failed with status: ($status)"
    exit 1
}

# Check main.rs has #![language(co2)]
let project_dir = ($test_dir | path join "co2cargo_init_test" "test_project")
let main_rs = ($project_dir | path join "src" "main.rs")
let content = (open $main_rs)
if not ($content | str contains "#![language(co2)]") {
    print $"main.rs missing #![language\(co2)]: ($content)"
    exit 1
}

# Check main.co2 file exists
let main_co2 = ($project_dir | path join "src/main.co2")
if not ($main_co2 | path exists) {
    print $"main.co2 does not exist at: ($main_co2)"
    exit 1
}

# Check main.co2 content
let co2_content = (open $main_co2)
if not ($co2_content | str contains "fn main()") {
    print $"main.co2 missing fn main(): ($co2_content)"
    exit 1
}

cd $project_dir
let status = (do { ^co2cargo -vv run } | complete)
if $status.exit_code != 0 {
    print $"Running co2cargo init generated code failed with status: ($status)"
    exit 1
}

print "co2cargo init binary test passed"
exit 0