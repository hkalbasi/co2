# Supported cargo commands and third party tools

Cargo has many builtin commands, and it allows `cargo-xxx` to act as `xxx` subcommand of cargo.
CO2, including `co2cargo`, tries hard to mimic Rust, so many of these commands are expected to work out of the box.
This document tracks commands and third party tools relation with CO2.

## Known to work

This list is tested to work at some time. If it doesn't work now, raise an issue so we either fix the problem or demote the tool.

* Main cargo actions `co2cargo build`, `co2cargo check`, `co2cargo run`, ... (+)
* `co2cargo doc` (+) and tools based on it:
    * `co2cargo semver-check`
    * `cheadergen` (You should use `RUSTDOC=co2rustdoc`)
* `co2cargo test` (+)
    * `co2cargo nextest`
* `co2cargo edit` commands like `co2cargo add`, `co2cargo update`, ...
* `co2cargo watch`
* `bacon` (with `cargo_json` analyzer):
```toml
[jobs.co2-check]
command = ["co2cargo", "check", "--message-format", "json-diagnostic-rendered-ansi"]
need_stdout = true
analyzer = "cargo_json"
```
* `co2cargo deb`
* `co2cargo llvm-cov` and other tools based on `-C instrument-coverage`
* Tools only about packages information
    * `co2cargo metadata`
    * `co2cargo tree`
    * `co2cargo vet`
    * `co2cargo deny`
    * `co2cargo outdated`
* `co2cargo flamegraph`
* `co2cargo publish` (but don't publish co2 code into crates.io, use git dependencies or custom registries for now)
* `co2cargo miri` (+)
* `co2cargo zigbuild`
* `co2cargo bloat`
* Debuggers like `gdb`, `lldb`, `rust-lldb`, ...

Note: Tools marked with (+) are tested in CI.

## Known broken

* `co2cargo fmt`: No formatter is implemented yet for CO2.
* `cbindgen`: Uses `syn` to parse Rust code. Use `cheadergen` instead which is rustdoc based.
* `bindgen`: Fundamentally not needed in CO2. Just `#include` the header you want.
* `co2cargo clippy`
* `co2cargo expand`: In theory it could work, needs `-Zunpretty=expanded` support in `co2rustc`. Meanwhile, use `co2cc -E` to expand macros.
* `co2cargo geiger`: There is no `unsafe` keyword in CO2, and everything is unsafe.
* Tools which need a helper dependency which requires macro
    * `cargo criterion`
    * `cargo fuzz`
* `cargo msrv`: Minimum supported Rust version of CO2 projects is the main branch of CO2 repo :).

## Unknown unknowns

Is a popular cargo tool missing from this list? Please test it with CO2 and make a PR.
