# Quick tour

## Installation

Install the CO2:

```
curl --proto '=https' --tlsv1.2 -sSf https://hkalbasi.github.io/co2/install.sh | sh
```

It installs `co2cc`, `co2rustc` and `co2cargo` in your PATH. See [installation page](./installation.md) for other installation methods.

## CO2 as a C compiler

The `co2cc` is a drop in replacement for a C compiler like gcc.
You can compile almost every (See [incompatibilities](./known_incompatibilities_with_c.md)) C code like:
```
co2cc foo.c
```
It tries to remain compatible with `gcc` in flags and CLI interface,
so you can use it as a C compiler in the build systems like cmake.

## Beyond C compiler

But CO2 is not a simple C compiler, and even `co2cc` is capable of compiling CO2 specific features.
Here is a tour over CO2 features:

```C
// Use statements can import from Rust std.
// They should be at the beginning of the file due parser technical limitations. 
use std::vec::Vec;
use std::option::Option;

// You can use rust primitives like i32 in C declarations.
int c_style_function(int a, i32 b) {
    return a + b;
}

// You can also define function signature using Rust syntax, to define a function with Rust ABI.
fn rust_style_function(a: i32, b: i32) -> i32 {
    // The body is still C.
    goto label;
    label:
        return c_style_function2(a, b); // Forward declarations are optional. 
}

#include <stdio.h>

i32 c_style_function2(int a, int b) {
    printf("c_style_function2(%d, %d)", a, b);
    return c_style_function(a, b);
}

int main() {
    Vec::<i32> v = Vec::<i32>::new(); // You can use generic Rust types and functions.
    for (int i = 0; i < 5; i++) {
        v.push(2 * i); // You can call Rust methods.
    }

    return 0;
}
```

## Integration with Rust

While with `co2cc` you can use the Rust standard library, for adding Rust dependencies,
and using the co2 code as a Rust crate, you need `co2cargo`.

Using `co2cargo init`, you can create a co2 binary crate. You can run it with `co2cargo run`,
add dependencies using `co2cargo add dep`, and almost all cargo commands which are supported.
Cargo commands which are dependent on a separate tool (like `cargo fmt`, `cargo clippy`, ...)
are not supported (see [exact list here](./cargo_third_party.md)).

## Next steps

* [Learn more about the language](./language_guide.md)
