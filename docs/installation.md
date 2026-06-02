# Installation

## Multicall packaged binary

The CO2 tools are currently packaged as a single busybox style multicall binary.
You can download the latest binary from the release page.
For installing it,
add symlinks named `co2cc`, `co2rustc` and `co2cargo` to it in your PATH.
Running `./co2-multicall install /usr/local/bin` will do this for you.

## Installation script

This script just automates the above:

```
curl --proto '=https' --tlsv1.2 -sSf https://hkalbasi.github.io/co2/install.sh | sh
```

## From source

This method is useful for changing the source code of the CO2 compiler, or frequently updating it.

1. Clone my rustc fork `https://github.com/hkalbasi/rust/`.
2. Checkout the `co2-changes` branch.
3. Install it as a toolchain called `co2`
4. Clone this repository
5. Build it using `cargo +co2 build -r`
6. `$env.LD_LIBRARY_PATH = (rustc --print sysroot)/lib`
7. Run `./target/release/co2-multicall install /usr/local/bin` to install it.
