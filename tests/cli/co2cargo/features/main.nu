# Test: co2cargo build with and without features

let test_dir = $env.CO2_TEST_DIR
let proj_dir = ($test_dir | path join "proj")

cd $proj_dir

# Run without any features
let without_feat = (do { ^co2cargo run } | complete)
if $without_feat.exit_code != 0 {
    print $"co2cargo run without features failed: ($without_feat.stderr)"
    exit 1
}
if not ($without_feat.stdout == "Hello world from CO2!\n") {
    print $"Unexpected output without features: ($without_feat.stdout)"
    exit 1
}

# Run with feat1 feature
let with_feat1 = (do { ^co2cargo run --features feat1 } | complete)
if $with_feat1.exit_code != 0 {
    print $"co2cargo run --features feat1 failed: ($with_feat1.stderr)"
    exit 1
}
if not ($with_feat1.stdout == "Hello world from CO2!\nfeat1 enabled\n") {
    print $"Unexpected output with feat1: ($with_feat1.stdout)"
    exit 1
}

# Run doc without features - neither function should appear
let doc_no_feat = (do {
    with-env { RUSTDOCFLAGS: "-Z unstable-options --output-format json" } {
        ^co2cargo doc --no-deps
    }
} | complete)
if $doc_no_feat.exit_code != 0 {
    print $"co2cargo doc without features failed: ($doc_no_feat.stderr)"
    exit 1
}

let json_path = ($proj_dir | path join "target" "doc" "proj.json")
let doc_data = (open --raw $json_path | from json)
let item_names = ($doc_data.index | values | each {|item| $item.name})

if ("feat1" in $item_names) {
    print "feat1 function should not appear in docs without features"
    exit 1
}
if ("feat_two" in $item_names) {
    print "feat_two function should not appear in docs without features"
    exit 1
}

# Run doc with feat1 feature - feat1 should appear, feat_two should not
let doc_with_feat1 = (do {
    with-env { RUSTDOCFLAGS: "-Z unstable-options --output-format json" } {
        ^co2cargo doc --no-deps --features feat1
    }
} | complete)
if $doc_with_feat1.exit_code != 0 {
    print $"co2cargo doc --features feat1 failed: ($doc_with_feat1.stderr)"
    exit 1
}

let doc_data_feat1 = (open --raw $json_path | from json)
let item_names_feat1 = ($doc_data_feat1.index | values | each {|item| $item.name})

if ("feat1" not-in $item_names_feat1) {
    print "feat1 function should appear in docs with feat1 feature"
    exit 1
}
if ("feat_two" in $item_names_feat1) {
    print "feat_two function should not appear in docs with feat1 feature only"
    exit 1
}

# Run with both feat1 and feat-two features
let with_both = (do { ^co2cargo run --features feat1,feat-two } | complete)
if $with_both.exit_code != 0 {
    print $"co2cargo run --features feat1,feat-two failed: ($with_both.stderr)"
    exit 1
}
if not ($with_both.stdout == "Hello world from CO2!\nfeat1 enabled\nfeat-two enabled\n") {
    print $"Unexpected output with both features: ($with_both.stdout)"
    exit 1
}

# Run doc with both features - both functions should appear
let doc_both = (do {
    with-env { RUSTDOCFLAGS: "-Z unstable-options --output-format json" } {
        ^co2cargo doc --no-deps --features feat1,feat-two
    }
} | complete)
if $doc_both.exit_code != 0 {
    print $"co2cargo doc --features feat1,feat-two failed: ($doc_both.stderr)"
    exit 1
}

let doc_data_both = (open --raw $json_path | from json)
let item_names_both = ($doc_data_both.index | values | each {|item| $item.name})

if ("feat1" not-in $item_names_both) {
    print "feat1 function should appear in docs with both features"
    exit 1
}
if ("feat_two" not-in $item_names_both) {
    print "feat_two function should appear in docs with both features"
    exit 1
}

print "co2cargo features test passed"
exit 0
