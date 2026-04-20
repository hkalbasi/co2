# Test co2cargo init for library crate
let test_dir = $env.CO2_TEST_DIR
mkdir ($test_dir | path join "co2cargo_init_lib_test") | ignore
cd ($test_dir | path join "co2cargo_init_lib_test")

# Run co2cargo init (co2cargo is in PATH via CO2_BIN_DIR)
let status = (do { ^co2cargo init test_lib --lib } | complete)
if $status.exit_code != 0 {
    print $"co2cargo init --lib failed with status: ($status)"
    exit 1
}

# Check lib.rs has #![co2::language]
let project_dir = ($test_dir | path join "co2cargo_init_lib_test" "test_lib")
let lib_rs = ($project_dir | path join "src" "lib.rs")
let content = (open $lib_rs)
if not ($content | str contains "#![co2::language]") {
    print $"lib.rs missing #![co2::language]: ($content)"
    exit 1
}

# Check lib.co2 file exists
let lib_co2 = ($project_dir | path join "src/lib.co2")
if not ($lib_co2 | path exists) {
    print $"lib.co2 does not exist at: ($lib_co2)"
    exit 1
}

# Check lib.co2 content
let co2_content = (open $lib_co2)
if not ($co2_content | str contains "fn add(") {
    print $"lib.co2 missing fn add(): ($co2_content)"
    exit 1
}

print "co2cargo init lib test passed"
exit 0