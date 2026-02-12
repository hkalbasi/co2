# Task

We implemented a way to use rustc emitting a custom fake crate with mir in the `rustc_public_generative`.
Now implement the CO2 compiler. CO2 is a C like language but supports Rust types and can import Rust
crates directly. This is an example CO2 code:

```co2
use std::fs::File;

void main() {
     File f = File::open("./foo");
     for (int i = 0; i < 100; i += 1) {
          f.do_something();
     }
}
```

You can learn the CO2 grammar from `./temp_ai/read_only/prior_art/chumsky_c_parser/`. You need to add
a binary crate `co2` which uses `rustc_public_generative` and behave as a normal `rustc`, except when
it detects a `#[language_co2]` in the Rust file, it tries to compile the `name.co2` file instead and
uses `rustc_public_generative` to generate the co2 code as a fake crate.

Use `chumsky` for your parser, like what I did in `prior_art/chumsky_c_parser`. You are allowed to
copy from my code as much as you want.

Add intermediate crates for parsing and generating hir and mir from co2 code. Except the `co2` crate, no
other crate in the workspace should depend on `rustc_public_generative` and `#![feature(rustc_private)]` and
they should work on stable.

For parsing CO2 code, use a two phase approach. First, parse the items and jump over bodies. In the second stage
parse each item throughly and use the data from dependencies and items of current crate to resolve names correctly. Remember
that C grammar is context sensitive and you need a resolver to correctly parse it. Unlike C, CO2 functions can see
functions and items defined below them and do not need forward declaration.

Then test your compiler on this code:
```co2
int write(int, char*, int);

void main() {
    write(1, "hello world\n", 12);
    write(1, "hello world\n", 12);
}
```
and make sure it works.

Finally, write a CO2 code that does the job of `fake_hello_world`. That is, read the arguments using `std::env::args()` and
open the file `argv[1]` and print its content. Test your compiler with that code.

## Conditions

* Read `./temp_ai/read_only/prior_art/`, it contains many things you need, including `azhdaha` which is
  a C compiler written in Rust by me. Try your HIR/MIR to be very similar to my design in `azhdaha`.
* A C grammar is in `./temp_ai/read_only/prior_art/chumsky_c_parser/grammar`. You can use it for C parts of the
  language. CO2 is just C with some additional Rust nicities.
* Follow my style, it is fine to copy code verbatimly from `prior_art`. But don't repeat my mistakes. (I generally
  don't make mistake but sometimes it happens).
* Finish the job as soon as possible, I want to comment on your design. Leave things with `todo!()` and just
  make the previous `fake_hello_world` example working, this time with a co2 code.
* Pay attention to spans. I want correct debug info. Load the co2 file using rustc infra so that it gets
  correct spans and also appear in the .d files emitted by the compiler.

## Notes
* I use JJ for development. You are not allowed to do Git/JJ commits, ask me and I will do that if I feel
  the code is in a good state. Don't run any JJ commands so that you don't accidentally break old approved
  code.
* Ask me whether you were in doubt. I know everything about this project and Rust in general better than you.
* Don't try to generate the perfect code in first try. Leave things with `todo!()` for future and just make
  the example working. But don't do hacky things and don't hard code examples in your compiler. Don't try
  to oversimplify things. Eventually I want a full compiler so if making things more complex
  make things easier to extend in future, choose the complex option. 
* You can use `cargo doc` for reading current workspace documentations, and I provided
  documentation of rustc private crates in `./temp_ai/read_only/rustc-private-docs`