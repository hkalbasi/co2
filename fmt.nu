#!/usr/bin/env nu

# stage 1: format workspace crates
cargo fmt

# stage 2: ensure every text file in project root ends with a newline
idx init . --wait
for f in (idx files .) {
    let f = $f.full_path
    let ft = (^file --brief --mime-type $f | str trim)
    if ($ft | str starts-with "text/") {
        let content = (open --raw $f)
        if not ($content | str ends-with "\n") {
            $content + "\n" | save --force $f
        }
    }
}
