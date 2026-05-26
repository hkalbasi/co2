#@ run-status: 0

let test_dir = $env.CO2_TEST_DIR
cd ($test_dir | path join "demo")

let result = (do { ^co2cargo doc --no-deps --document-private-items } | complete)
if $result.exit_code != 0 {
    print $"expected successful co2cargo doc run, got: ($result | to json -r)"
    exit 2
}

let crate_index = ($test_dir | path join "demo" "target" "doc" "demo" "index.html")
if (($crate_index | path exists) == false) {
    print $"missing crate index: ($crate_index)"
    exit 3
}

let crate_html = (open --raw $crate_index)
if ($crate_html | str contains "documented") == false {
    print $"missing documented function in crate docs: ($crate_html)"
    exit 4
}

let module_index = ($test_dir | path join "demo" "target" "doc" "demo" "math" "index.html")
if (($module_index | path exists) == false) {
    print $"missing module index: ($module_index)"
    exit 5
}

let module_html = (open --raw $module_index)
if ($module_html | str contains "nested") == false {
    print $"missing nested function in module docs: ($module_html)"
    exit 6
}

let source_dir = ($test_dir | path join "demo" "target" "doc" "src")
if (($source_dir | path exists) == false) {
    print $"missing source output directory: ($source_dir)"
    exit 7
}

let source_listing = (do { ^find $source_dir -type f -name '*.html' } | complete)
if $source_listing.exit_code != 0 {
    print $"failed to list source html files: ($source_listing | to json -r)"
    exit 8
}

let lib_source = (
    $source_listing.stdout
    | lines
    | where {|path| $path | str ends-with 'lib.co2.html'}
    | first
)
if ($lib_source | is-empty) {
    print $"missing lib.co2 source page under: ($source_dir)"
    exit 9
}

let lib_source_html = (open --raw $lib_source)
if (
    ($lib_source_html | str contains "documented() -&gt; i32") == false
    or ($lib_source_html | str contains "<span class=\"number\">42</span>") == false
) {
    print $"missing co2 source content in source page: ($lib_source_html)"
    exit 10
}
