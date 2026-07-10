# CO2

CO2 (oxidized C) is a programming language which is backward compatible with
C (See [incompatibilities](./docs/known_incompatibilities_with_c.md)) but with
direct access to the Rust ecosystem. CO2 and Rust can use each other crates seamlessly,
with no FFI boundaries or extra tooling required. 

## Example

```C++
use std::vec::Vec;
use std::f64::consts::PI;

// You can include any system installed header.
// You can manage include directories using build.rs.
#include <stdio.h>

// C-style function with C ABI
int add(int a, int b) {
    return a + b;
}

typedef int (*F)(int);

int all_c_is_valid(int x)
{
    enum { A = 1 };
    struct S { unsigned b:3; int a[2]; };
    union U { struct S s; int i; };

    volatile const union U u = { .s = { .b = 3, .a = {1, 2} } };
    F f = (F)0;
    int y = 0, a[] = {3, 4}, *p = a;

again:
    for (int i = 0; i < 2; i++)
        y += (x > i) ? p[i] : 0;

    switch (x) {
    case 0: y += sizeof(u); break;
    default:
        if (f)
            y = f(y);
        else
            y = ((int)u.s.b, y + *p++);
    }

    if (x--)
        goto again;

    return y;
}

// Rust-style function with Rust ABI
fn rust_multiply(a: i32, b: i32) -> i32 {
    // The body still uses the C like syntax. E.g. using `a * b` without return is invalid.
    return a * b;
}

fn main() {
    // C-style function call
    int sum = add(3, 4);

    // Calling C external functions does not need `unsafe` block.
    // In fact, there is no unsafe block at all.
    printf("Sum is %d\n", sum);
    printf("all_c_is_valid(2) is %d\n", all_c_is_valid(2));

    // Rust-style function call (Rust ABI)
    i32 product = rust_multiply(5, 6);
    printf("Product is %d\n", product);

    // Using generic Rust types
    Vec::<i32> v = Vec::<i32>::new();
    v.push(10);
    v.push(20);

    // Access Rust method results
    i32 last = v.pop().unwrap();
    printf("Last is %d\n", last);

    // Use Rust constants
    double pi = PI;
    printf("PI is %.8f\n", pi);

    v.push(20);
    v.push(3);
    v.push(15);

    // Raw pointers get automatically casted to references.
    // There is a borrow checker which can see through the raw pointers
    // and warn you when things are obviously wrong, but the programmer
    // is solely responsible for avoiding UB.
    Vec::<i32> *v_ptr = &v;

    v_ptr.push(1000); // A customized auto dereference mechanism is available.
    Vec::<i32>::push(&v, 60);
    v.push(16);

    // C style for loop
    i32 *v_data_ptr = v.as_ptr();
    for (int i = 0; i < v.len(); i += 1) {
        printf("%d ", v_data_ptr[i]);
    }
    printf("\n");
}
```

You can run [this example online on godbolt.org](https://godbolt.org/z/vK6KE1zzh).

## Why another language?

C has served systems programming for decades, but its lack of memory safety, minimal type system,
and absence of a standard package manager are increasingly making people look for alternatives.
Rust has emerged as one of the leading candidate to fill this role,
offering memory safety without a garbage collector, a rich type system, and a modern build system / package manager.

But Rust has a not great story in integrating with C:
* C libraries are second class citizens
  * The API becomes an unsafe and unidiomatic Rust, which probably needs a wrapper
  * They are not distributed by Cargo, and often distributed in binary form, limiting things like LTO or cross compilation
* Rust libraries are not usable in C, needing an special C wrapper
  * Losing Rust types and falling back to things like `void*`
  * Some things (like `HashMap`) are impossible to use in C without performance loss
  * A C wrapper is time consuming to develop

These issues lead to a trend of RIIR (Rewrite It In Rust) which is taking an existing C or C++ project and rewriting it entirely
in Rust to gain safety and modern tooling. This may work for small projects or organizations with massive resources,
but there are billions of lines of C code which is not going to be rewritten, any time soon (unless AI advances massively).

CO2 fixes these issues. Instead of rewriting your project in Rust, you can rewrite it in CO2.
A CO2 rewrite is an easy and feasible task, since CO2 is backward compatible with C,
so it is not like a full rewrite, its more like changing a build system. See [co2-quickjs](https://github.com/HKalbasi/co2-quickjs/)
as an example of a non trivial project ported to CO2.

After that, your CO2 code is a crate like any other Rust crates. You can import Rust crates without any change,
use all of their API (even generic types and functions), split your code into multiple CO2 crates,
and rewriting some of them or incrementally all of them to Rust for safety gains.

Even if you don't need Rust interop, and you just want a better C with some features, 
[CO2 might address your needs](./docs/c_successor.md).

## Getting started

Install the CO2:

```
curl --proto '=https' --tlsv1.2 -sSf https://hkalbasi.github.io/co2/install.sh | sh
```

You can use CO2 in two ways:

* As a standalone C compiler: `co2cc` is a drop-in replacement for gcc, compatible in flags and CLI interface. You can use it directly or plug it into build systems like CMake. It compiles standard C, but also enables some CO2-specific extensions and use of the Rust standard library.

* As a Cargo-based project: `co2cargo` wraps Rust's cargo, letting you create co2 binary and library crates dependent on Rust crates, and vise versa. Run `co2cargo init` to scaffold a project, `co2cargo add <dep>` to pull in Rust crates, and `co2cargo run` to build and execute (See [all available commands](./docs/cargo_third_party.md)). The two languages share a common compiler backend, so calls between them carry zero overhead, and the syntax added to C by CO2 provides a seamless experience in using Rust crates with their original API.

For a step-by-step walkthrough covering installation, language features, and project setup, see the [Quick Tour](./docs/quick_tour.md).

## Contributing

See [contributing docs](./docs/contributing/README.md).

## FAQ

### Is CO2 a memory safe language?

No. It's a safer language than C, but is not a safe language like Rust.

It is safer than C because:
* CO2 code has access to fat pointers, smart pointers and Rust containers,
  which enables bound checking and other safety guarantees if used.
* Some kind of borrow checker is still running, which kicks in specially when using Rust code wrongly.
  The borrow checker only emits warning, and the programmer is responsible for everything,
  but it can still be useful in detecting memory bugs.
* It enables using safe Rust dependencies, with its original API, not an unsafe FFI wrapper around it.
* It enables rewriting safety critical parts of code to Rust, which is memory safe.

But it is not a safe language like Rust:
* There is no `unsafe` block (You can declare your functions as unsafe but it is only for your
  Rust dependents). CO2 allows you to freely dereferencing raw pointers coming from C, calling unsafe Rust functions,
  using unions, and many other things which can easily cause UB.
* You can freely call every C function, e.g. all libc functions, which most of them are super unsafe,
  and have non-trivial preconditions that the caller should abide for memory safety.

### Is C++ syntax supported?

No. We can imagine a CO2++ language which is backward compatible with C++. But CO2 is just compatible with C.
For more details on CO2++, see [this page](./docs/vision/co2pp.md). 

### Why the compiler binary is so huge?

The currently packaged binary is a self extracting archive of co2 + rustc + rust std + llvm + miri.
It would be much smaller if you install the rustc separately and [build CO2 from source](./docs/installation.md).

### Is every mix of Rust and C syntax accepted? Can I assume that I know CO2 if I know C and Rust?

No. CO2 tries to accept every C syntax, but it doesn't try to allow all Rust syntaxes.
CO2 doesn't want to change C fundamentally, it only wants to provide seamless interop between C and Rust crates.
Sometimes this interop needs additional syntax, and in those cases, CO2 first looks at the Rust for inspiration,
to make things familiar for the target audience, but even the imported syntaxes from Rust may have subtle differences.

If you already know C and Rust, you are very close. Just read [the language guide](./docs/language_guide.md) (~5 min) and you are done.
By just looking at the examples, you may get some expectations that doesn't match reality.
The compiler will stop you, but the error messages are not as high quality as Rust, so you may need to read the docs.

### How much LLM is used in developing CO2?

See [LLM Policy](./docs/contributing/llm_policy.md) which also explains this.

### Isn't C/Rust interop a solved problem? Why you don't focus on the better C part?

No. If C/Rust interop was a completely solved problem, you could use all Rust's standard library in C,
so you had most of the CO2 benefits in the C itself. If you consider CO2 a better C,
it is just because of its Rust interop, and without Rust interop CO2 is just C.

By that definition, the ultimate C/Rust FFI solution certainly needs language changes on the C side,
and CO2 tries to be that solution. Having access to the Rust standard library, Rust ecosystem and Rust tools are a great feature,
and by enabling them, CO2 becomes a better C in that process.

### Why CO2 doesn't let me defining generic structs and functions?

CO2 doesn't want to change the identity of C, and adding major features like generics/templates which C doesn't have,
is against that. The goal is to keep CO2 close to C mental model as much as possible, to keep it familiar for C programmers.

If you are adding generic structs and functions, you are not thinking in C. CO2 enables you to add those generic items in Rust crates,
and use them in your CO2 code seamlessly.

### How to rewrite a C project in Rust using CO2?

CO2 enables incrementally migrating projects between C and Rust, by using CO2 as a midpoint.
For converting a C project to Rust, you can:

1. Convert it to a co2 project
2. Use Rust dependencies instead of C
  1. Use Rust std instead of libc
  2. Use crates.io instead of hand coded things
  3. Use Rust wrappers of C dependencies instead of themselves.
3. Split your project into multiple crates
  1. Make the public API of each crate minimal and idiomatic in Rust
4. Rewrite each crate into Rust, one by one.
  1. Use tests to keep the behavior stable in each rewrite.

## License

CO2 is distributed under the terms of both the MIT license and the Apache License (Version 2.0).
